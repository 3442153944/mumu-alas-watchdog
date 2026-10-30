"""运行状态采集、status.json 写入与状态日志格式化。"""

from __future__ import annotations

from datetime import datetime
from typing import Any, Dict, List, Optional

from .config import Settings
from .constants import STATUS_JSON, VERSION
from .layout import Layout
from .util import atomic_write_json, fmt_duration, now_text
from .win32api import proc_create_time, proc_memory


def build_status(
    layout: Layout,
    mumu,
    alas,
    logwatch,
    *,
    next_restart: Optional[datetime],
    cycle_count: int,
    failures: int,
    last_cycle: Optional[dict],
    started_at: datetime,
) -> Dict[str, Any]:
    status: Dict[str, Any] = {
        "time": now_text(),
        "watchdog": {
            "pid": __import__("os").getpid(),
            "version": VERSION,
            "uptime_seconds": int((datetime.now() - started_at).total_seconds()),
            "next_restart": next_restart.strftime("%Y-%m-%d %H:%M:%S") if next_restart else None,
            "cycle_count": cycle_count,
            "failures": failures,
            "last_cycle": last_cycle,
        },
        "emulator": {},
        "adb": {},
        "alas": {},
    }

    try:
        info = mumu.info(timeout=20)
        emulator = {
            "name": info.get("name"),
            "index": info.get("index"),
            "state": info.get("player_state"),
            "is_process_started": bool(info.get("is_process_started")),
            "is_android_started": bool(info.get("is_android_started")),
            "pid": info.get("pid"),
            "adb_port": info.get("adb_port"),
        }
        pid = info.get("pid")
        if pid:
            working_set, private = proc_memory(int(pid))
            emulator["working_set_gb"] = round(working_set / 2 ** 30, 2)
            emulator["commit_gb"] = round(private / 2 ** 30, 2)
            created = proc_create_time(int(pid))
            if created:
                emulator["started_at"] = created.strftime("%Y-%m-%d %H:%M:%S")
                emulator["uptime_seconds"] = int((datetime.now() - created).total_seconds())
        status["emulator"] = emulator
    except Exception as exc:
        status["emulator"] = {"error": str(exc)}

    try:
        state, response_ms = mumu.timed_adb_state()
        status["adb"] = {
            "serial": mumu.serial,
            "state": state,
            "response_ms": int(response_ms),
        }
    except Exception as exc:
        status["adb"] = {"serial": mumu.serial, "error": str(exc)}

    try:
        main = alas.electron_main()
        webui_ok, webui_ms = alas.webui_status()
        task_info = logwatch.status_dict()
        status["alas"] = {
            "electron_pid": main.pid if main else None,
            "electron_processes": len(alas.electron_processes()),
            "webui_ok": webui_ok,
            "webui_response_ms": int(webui_ms),
            "webui_port": layout.webui_port,
            "current_task": task_info.get("current_task"),
            "task_info": task_info,
            "manual_stopped": logwatch.manual_stopped,
        }
    except Exception as exc:
        status["alas"] = {"error": str(exc)}
    return status


def write_status(settings: Settings, payload: Dict[str, Any]) -> None:
    if settings.write_status_json:
        atomic_write_json(STATUS_JSON, payload)


def format_status_lines(status: Dict[str, Any], next_restart: Optional[datetime]) -> List[str]:
    lines: List[str] = []
    emulator = status.get("emulator", {})
    if emulator.get("error"):
        lines.append(f"[状态] 模拟器查询失败: {emulator.get('error')}")
    else:
        lines.append(
            "[状态] 模拟器: state={state} pid={pid} 工作集={ws}GB 提交={mem}GB 已运行={uptime}".format(
                state=emulator.get("state"),
                pid=emulator.get("pid"),
                ws=emulator.get("working_set_gb", "?"),
                mem=emulator.get("commit_gb", "?"),
                uptime=fmt_duration(emulator.get("uptime_seconds", 0)),
            )
        )
    adb = status.get("adb", {})
    alas = status.get("alas", {})
    if adb.get("error"):
        lines.append(f"[状态] ADB 查询失败: {adb.get('error')}")
    else:
        lines.append(
            f"[状态] ADB: {adb.get('state') or '无响应'}（{adb.get('response_ms')} ms）"
            f" | WebUI: {'正常' if alas.get('webui_ok') else '异常'}"
            f"（{alas.get('webui_response_ms')} ms）"
        )
    if alas.get("current_task"):
        lines.append(
            f"[状态] Alas: 当前任务 {alas.get('current_task')}，"
            f"最近日志 {alas.get('task_info', {}).get('last_activity')}"
        )
    else:
        lines.append(f"[状态] Alas: 空闲/等待任务 | Electron PID={alas.get('electron_pid')}")
    lines.append(
        f"[状态] 计划: 下次重启 {next_restart:%Y-%m-%d %H:%M:%S}"
        if next_restart
        else "[状态] 计划: 尚未安排"
    )
    return lines


__all__ = ["build_status", "write_status", "format_status_lines"]
