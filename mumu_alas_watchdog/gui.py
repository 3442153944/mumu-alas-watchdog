"""tkinter GUI 控制台：状态展示、启停/重启按钮、日志轮询、操作输出。"""

from __future__ import annotations

import json
import os
import queue
import subprocess
import tempfile
import threading
from datetime import datetime
from pathlib import Path
from typing import Dict, List

from .config import Settings
from .constants import APP_NAME, CREATE_NO_WINDOW, STATUS_JSON, VERSION, WD_DIR
from .lockfile import watcher_running_pid
from .util import fmt_duration, now_text, self_command, tail_lines
from .watchdog import Watchdog


def _open_path(path: Path) -> None:
    try:
        os.startfile(str(path))  # type: ignore[attr-defined]
    except Exception:
        pass


def run_gui(settings: Settings) -> int:
    import tkinter as tk
    from tkinter import messagebox, scrolledtext, ttk

    root = tk.Tk()
    root.title(f"{APP_NAME} v{VERSION} · MuMu + Alas 自动重启控制台")
    root.geometry("1020x700")
    root.minsize(860, 620)
    try:
        ttk.Style(root).theme_use("vista")
    except tk.TclError:
        pass

    ui_queue: "queue.Queue[tuple]" = queue.Queue()
    stop_event = threading.Event()
    refresh_event = threading.Event()

    status_frame = ttk.LabelFrame(root, text="运行状态（每 2 秒自动刷新）", padding=(10, 6))
    status_frame.pack(fill="x", padx=10, pady=(10, 4))
    fields = [
        ("watchdog", "看门狗"),
        ("next_restart", "下次重启"),
        ("emulator", "模拟器"),
        ("adb", "ADB"),
        ("alas", "Alas"),
        ("last_cycle", "最近重启"),
    ]
    status_vars: Dict[str, "tk.StringVar"] = {}
    for row, (key, label) in enumerate(fields):
        ttk.Label(status_frame, text=f"{label}：", width=10, anchor="e").grid(
            row=row, column=0, sticky="e", pady=1
        )
        variable = tk.StringVar(value="读取中 ...")
        status_vars[key] = variable
        ttk.Label(status_frame, textvariable=variable, anchor="w").grid(
            row=row, column=1, sticky="w", pady=1
        )
    status_frame.columnconfigure(1, weight=1)

    button_frame = ttk.Frame(root)
    button_frame.pack(fill="x", padx=10, pady=4)
    buttons: List["ttk.Button"] = []

    def add_button(text: str, command, width: int = 16) -> "ttk.Button":
        button = ttk.Button(button_frame, text=text, command=command, width=width)
        button.pack(side="left", padx=3, pady=2)
        buttons.append(button)
        return button

    log_frame = ttk.Frame(root)
    log_frame.pack(fill="both", expand=True, padx=10, pady=(4, 8))
    notebook = ttk.Notebook(log_frame)
    notebook.pack(fill="both", expand=True)
    log_tab = ttk.Frame(notebook)
    output_tab = ttk.Frame(notebook)
    notebook.add(log_tab, text="看门狗日志")
    notebook.add(output_tab, text="操作输出")
    log_box = scrolledtext.ScrolledText(log_tab, height=18, wrap="none", font=("Consolas", 9))
    log_box.pack(fill="both", expand=True)
    log_box.configure(state="disabled")
    output_box = scrolledtext.ScrolledText(
        output_tab, height=18, wrap="word", font=("Consolas", 9)
    )
    output_box.pack(fill="both", expand=True)
    output_box.configure(state="disabled")

    status_var = tk.StringVar(value="就绪")
    ttk.Label(root, textvariable=status_var, anchor="w", relief="sunken", padding=(6, 2)).pack(
        fill="x", side="bottom"
    )

    def append_output(text: str) -> None:
        output_box.configure(state="normal")
        output_box.insert("end", text.rstrip() + "\n\n")
        output_box.see("end")
        output_box.configure(state="disabled")

    def set_busy(description: str) -> None:
        status_var.set(description)
        for button in buttons:
            button.state(["disabled"])

    def clear_busy() -> None:
        for button in buttons:
            button.state(["!disabled"])
        status_var.set("就绪")

    def run_command(args: List[str], description: str) -> None:
        temporary = tempfile.NamedTemporaryFile(
            prefix="muw_cmd_", suffix=".log", delete=False, dir=str(WD_DIR)
        )
        temporary_path = temporary.name
        temporary.close()
        environment = dict(os.environ)
        environment["MUW_CMD_LOG"] = temporary_path
        environment.pop("ELECTRON_RUN_AS_NODE", None)
        command = self_command(args)
        set_busy(f"正在执行：{description} ...")
        notebook.select(output_tab)
        append_output(f"[{now_text()}] {description}")

        def worker() -> None:
            code = -1
            try:
                completed = subprocess.run(
                    command,
                    cwd=str(WD_DIR),
                    env=environment,
                    timeout=1800,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    creationflags=CREATE_NO_WINDOW,
                )
                code = completed.returncode
            except Exception as exc:
                ui_queue.put(("cmd_output", f"执行失败: {exc}"))
            try:
                output = Path(temporary_path).read_text(encoding="utf-8", errors="replace")
            except OSError:
                output = ""
            try:
                Path(temporary_path).unlink()
            except OSError:
                pass
            ui_queue.put(("cmd_done", (description, code, output)))

        threading.Thread(target=worker, daemon=True).start()

    def monitor_loop() -> None:
        try:
            watchdog = Watchdog(settings, to_file_log=False, console=False)
        except Exception as exc:
            ui_queue.put(("monitor_error", f"读取状态失败: {exc}"))
            return
        while not stop_event.is_set():
            try:
                texts = watchdog.logwatch.scan()
                watchdog.logwatch.analyze(texts)
                live = watchdog.collect_status()
                running = watcher_running_pid()
                daemon_status = None
                if running:
                    try:
                        data = json.loads(STATUS_JSON.read_text(encoding="utf-8"))
                        if int(data.get("watchdog", {}).get("pid") or 0) == int(running):
                            daemon_status = data
                    except Exception:
                        daemon_status = None
                ui_queue.put(("status", (running, live, daemon_status, watchdog)))
            except Exception as exc:
                ui_queue.put(("monitor_error", f"刷新状态失败: {exc}"))
            refresh_event.wait(2.0)
            refresh_event.clear()

    def apply_status(payload) -> None:
        running, live, daemon_status, watchdog = payload
        status_vars["watchdog"].set(f"运行中（PID {running}）" if running else "未运行")

        schedule_source = daemon_status or live
        watchdog_info = schedule_source.get("watchdog", {}) if isinstance(schedule_source, dict) else {}
        next_restart = watchdog_info.get("next_restart")
        if next_restart:
            suffix = "（看门狗计划）" if daemon_status else "（估算）"
            status_vars["next_restart"].set(f"{next_restart}{suffix}")
        else:
            try:
                estimate = watchdog.compute_next_restart(datetime.now())
                status_vars["next_restart"].set(f"{estimate:%Y-%m-%d %H:%M:%S}（估算）")
            except Exception:
                status_vars["next_restart"].set("-")

        emulator = live.get("emulator", {}) or {}
        if emulator.get("error"):
            status_vars["emulator"].set(f"查询失败：{emulator.get('error')}")
        elif emulator.get("is_process_started") and emulator.get("is_android_started"):
            status_vars["emulator"].set(
                "运行中 · {state} · PID {pid} · 内存 {ws}/{commit} GB · 已运行 {uptime}".format(
                    state=emulator.get("state"),
                    pid=emulator.get("pid"),
                    ws=emulator.get("working_set_gb", "?"),
                    commit=emulator.get("commit_gb", "?"),
                    uptime=fmt_duration(emulator.get("uptime_seconds", 0)),
                )
            )
        else:
            status_vars["emulator"].set(f"未运行（{emulator.get('state') or '未知'}）")

        adb = live.get("adb", {}) or {}
        if adb.get("error"):
            status_vars["adb"].set(f"查询失败：{adb.get('error')}")
        else:
            status_vars["adb"].set(f"{adb.get('state') or '无响应'}（响应 {adb.get('response_ms', '?')} ms）")

        alas = live.get("alas", {}) or {}
        if alas.get("error"):
            status_vars["alas"].set(f"查询失败：{alas.get('error')}")
        else:
            task = alas.get("current_task") or "空闲/等待中"
            webui = "正常" if alas.get("webui_ok") else "异常"
            status_vars["alas"].set(
                f"WebUI {webui}（{alas.get('webui_response_ms', '?')} ms） · "
                f"当前任务 {task} · Electron PID {alas.get('electron_pid')}"
            )

        cycle = None
        if daemon_status:
            cycle = daemon_status.get("watchdog", {}).get("last_cycle")
        if isinstance(cycle, dict):
            result = "成功" if cycle.get("ok") else "失败"
            status_vars["last_cycle"].set(
                f"{cycle.get('time')} {result}（{cycle.get('reason')}，{cycle.get('duration_seconds')} 秒）"
            )
        else:
            status_vars["last_cycle"].set("暂无（看门狗未运行或尚未重启过）")

    def refresh_log_panel() -> None:
        try:
            candidates = sorted(
                settings.log_dir.glob("watchdog_*.log"),
                key=lambda item: item.stat().st_mtime if item.exists() else 0,
                reverse=True,
            )
            if candidates:
                text = "\n".join(tail_lines(candidates[0], 200))
                if text != log_box.get("1.0", "end-1c"):
                    log_box.configure(state="normal")
                    log_box.delete("1.0", "end")
                    log_box.insert("end", text)
                    log_box.see("end")
                    log_box.configure(state="disabled")
        except Exception:
            pass
        root.after(2000, refresh_log_panel)

    def poll_ui_queue() -> None:
        try:
            while True:
                kind, payload = ui_queue.get_nowait()
                if kind == "status":
                    apply_status(payload)
                elif kind == "cmd_output":
                    append_output(str(payload))
                elif kind == "cmd_done":
                    description, code, output = payload
                    append_output(f"[{now_text()}] {description} 结束（返回码 {code}）")
                    if str(output).strip():
                        append_output(str(output))
                    clear_busy()
                elif kind == "monitor_error":
                    status_var.set(str(payload))
        except queue.Empty:
            pass
        root.after(200, poll_ui_queue)

    def confirm(description: str) -> bool:
        return messagebox.askyesno(APP_NAME, f"{description}\n\n会中断当前 Alas 任务，确定继续吗？")

    def restart_full() -> None:
        if confirm("立即执行完整重启（MuMu 模拟器 + Alas）？"):
            run_command(["--restart-now"], "完整重启（模拟器 + Alas）")

    def restart_alas() -> None:
        if confirm("立即重启 Alas（不重启模拟器）？"):
            run_command(["--restart-alas-now"], "只重启 Alas")

    add_button("启动看门狗", lambda: run_command(["--start"], "启动看门狗"), width=13)
    add_button("停止看门狗", lambda: run_command(["--stop"], "停止看门狗"), width=13)
    add_button("立即完整重启", restart_full, width=13)
    add_button("只重启 Alas", restart_alas, width=13)
    add_button("刷新状态", lambda: (refresh_event.set(), status_var.set("正在刷新 ...")), width=10)
    add_button("打开日志目录", lambda: _open_path(settings.log_dir), width=12)
    add_button("打开配置文件", lambda: _open_path(settings.path), width=12)
    add_button("安装开机自启", lambda: run_command(["--install-autostart"], "安装开机自启"), width=14)
    add_button("移除开机自启", lambda: run_command(["--uninstall-autostart"], "移除开机自启"), width=14)

    def on_close() -> None:
        stop_event.set()
        refresh_event.set()
        root.destroy()

    root.protocol("WM_DELETE_WINDOW", on_close)
    status_var.set("就绪：看门狗未运行时可查看状态；点“启动看门狗”后后台常驻")
    refresh_log_panel()
    threading.Thread(target=monitor_loop, daemon=True).start()
    root.after(200, poll_ui_queue)
    root.mainloop()
    stop_event.set()
    return 0


__all__ = ["run_gui"]
