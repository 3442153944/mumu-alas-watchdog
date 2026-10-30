"""通用工具：命令执行、时间/路径、HTTP/端口探测、日志尾部读取、进程启动命令。"""

from __future__ import annotations

import json
import logging
import os
import re
import shutil
import socket
import subprocess
import sys
import time
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, Iterable, List, Optional, Tuple

from .constants import APP_NAME, APP_PATH, CONFIG_PATH, CREATE_NO_WINDOW, FROZEN, WD_DIR


@dataclass
class RunResult:
    rc: int
    out: str
    err: str
    timed_out: bool = False
    cmd: Tuple[str, ...] = ()


def decode_bytes(data: Optional[bytes]) -> str:
    if not data:
        return ""
    for encoding in ("utf-8", "gbk", "mbcs"):
        try:
            return data.decode(encoding)
        except Exception:
            continue
    return data.decode("utf-8", "replace")


def run_cmd(
    cmd: Iterable[object], timeout: float = 60, cwd: Optional[Path] = None
) -> RunResult:
    """执行命令并捕获输出（不弹出黑框）。"""
    cmd_list = [str(item) for item in cmd]
    try:
        completed = subprocess.run(
            cmd_list,
            cwd=str(cwd) if cwd else None,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout,
            creationflags=CREATE_NO_WINDOW,
        )
        return RunResult(
            rc=int(completed.returncode),
            out=decode_bytes(completed.stdout),
            err=decode_bytes(completed.stderr),
            cmd=tuple(cmd_list),
        )
    except subprocess.TimeoutExpired as exc:
        return RunResult(
            rc=-1,
            out=decode_bytes(exc.stdout if isinstance(exc.stdout, bytes) else None),
            err=decode_bytes(exc.stderr if isinstance(exc.stderr, bytes) else None),
            timed_out=True,
            cmd=tuple(cmd_list),
        )
    except OSError as exc:
        return RunResult(rc=-2, out="", err=str(exc), cmd=tuple(cmd_list))


def kill_pid_tree(pid: int) -> None:
    try:
        subprocess.run(
            ["taskkill", "/F", "/T", "/PID", str(pid)],
            timeout=30,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            creationflags=CREATE_NO_WINDOW,
        )
    except Exception:
        pass


def now_text() -> str:
    from datetime import datetime

    return datetime.now().strftime("%Y-%m-%d %H:%M:%S")


def fmt_duration(seconds: float) -> str:
    seconds = int(max(0, seconds))
    hours, rem = divmod(seconds, 3600)
    minutes, secs = divmod(rem, 60)
    if hours:
        return f"{hours}小时{minutes}分{secs}秒"
    if minutes:
        return f"{minutes}分{secs}秒"
    return f"{secs}秒"


def norm_path(value: os.PathLike) -> str:
    try:
        return os.path.normcase(os.path.abspath(str(value)))
    except Exception:
        return str(value)


def same_path(a: os.PathLike, b: os.PathLike) -> bool:
    return norm_path(a) == norm_path(b)


def path_is_under(child: os.PathLike, base: os.PathLike) -> bool:
    child_s = norm_path(child)
    base_s = norm_path(base)
    return child_s == base_s or child_s.startswith(base_s + os.sep)


def ps_quote(text: str) -> str:
    return "'" + text.replace("'", "''") + "'"


def port_listening(port: int, host: str = "127.0.0.1", timeout: float = 1.0) -> bool:
    try:
        with socket.create_connection((host, int(port)), timeout=timeout):
            return True
    except OSError:
        return False


def http_probe(port: int, timeout: float = 4.0) -> Tuple[bool, float]:
    """返回 (WebUI 是否可访问, 响应毫秒)。"""
    url = f"http://127.0.0.1:{int(port)}/"
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    started = time.time()
    try:
        with opener.open(url, timeout=timeout) as resp:
            ok = 200 <= int(getattr(resp, "status", 200)) < 500
    except Exception:
        logging.getLogger(__name__).debug("HTTP 探测失败: %s", url, exc_info=True)
        ok = False
    return ok, (time.time() - started) * 1000


def parse_hhmm_list(value) -> List[Tuple[int, int]]:
    if isinstance(value, str):
        parts = re.split(r"[,;\s]+", value)
    elif isinstance(value, (list, tuple)):
        parts = [str(item) for item in value]
    else:
        parts = []
    result: List[Tuple[int, int]] = []
    for part in parts:
        item = str(part).strip().strip('"').strip("'")
        match = re.fullmatch(r"(\d{1,2}):(\d{2})", item)
        if not match:
            continue
        hour, minute = int(match.group(1)), int(match.group(2))
        if 0 <= hour < 24 and 0 <= minute < 60:
            result.append((hour, minute))
    return sorted(set(result))


def tail_lines(path: Path, max_lines: int = 300, max_bytes: int = 768 * 1024) -> List[str]:
    try:
        with open(path, "rb") as handle:
            handle.seek(0, os.SEEK_END)
            size = handle.tell()
            handle.seek(max(0, size - max_bytes))
            data = handle.read()
        return data.decode("utf-8", "replace").splitlines()[-max_lines:]
    except OSError:
        return []


def last_line_with(path: Path, needle: str) -> Optional[str]:
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return None
    for line in reversed(text.splitlines()):
        if needle in line:
            return line.strip()
    return None


def atomic_write_json(path: Path, payload: dict) -> None:
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        temporary = path.with_suffix(path.suffix + ".tmp")
        temporary.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")
        os.replace(str(temporary), str(path))
    except OSError:
        pass


def pythonw_path() -> Optional[str]:
    """返回与当前解释器同目录的 pythonw.exe（后台无窗口运行）。"""
    executable = Path(sys.executable)
    if executable.name.lower() == "pythonw.exe":
        return str(executable)
    candidate = executable.with_name("pythonw.exe")
    if candidate.exists():
        return str(candidate)
    return shutil.which("pythonw")


def self_command(args: List[str]) -> List[str]:
    """返回“再启动自身”的命令行（源码模式用 python -m 包名）。"""
    if FROZEN:
        return [str(APP_PATH)] + list(args)
    return [sys.executable, "-m", "mumu_alas_watchdog"] + list(args)


def background_command(config_path: Path) -> Optional[List[str]]:
    if FROZEN:
        command = [str(APP_PATH), "--daemon"]
    else:
        pythonw = pythonw_path()
        if not pythonw:
            return None
        command = [pythonw, "-m", "mumu_alas_watchdog", "--daemon"]
    try:
        if Path(config_path).resolve() != CONFIG_PATH.resolve():
            command += ["--config", str(config_path)]
    except OSError:
        pass
    return command


__all__ = [
    "RunResult",
    "decode_bytes",
    "run_cmd",
    "kill_pid_tree",
    "now_text",
    "fmt_duration",
    "norm_path",
    "same_path",
    "path_is_under",
    "ps_quote",
    "port_listening",
    "http_probe",
    "parse_hhmm_list",
    "tail_lines",
    "last_line_with",
    "atomic_write_json",
    "pythonw_path",
    "self_command",
    "background_command",
    "WD_DIR",
    "APP_NAME",
]
