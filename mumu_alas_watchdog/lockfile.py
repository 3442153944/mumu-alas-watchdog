"""看门狗单实例锁与 PID 文件。"""

from __future__ import annotations

import os
from pathlib import Path
from typing import Optional

from .constants import PID_FILE
from .win32api import is_pid_alive


def acquire_pid_lock(path: Path = PID_FILE) -> Optional[int]:
    """独占创建 PID 文件；已存在且进程存活时返回 None。"""
    for _ in range(2):
        try:
            handle = os.open(str(path), os.O_CREAT | os.O_EXCL | os.O_WRONLY)
        except FileExistsError:
            pid = 0
            try:
                pid = int(path.read_text(encoding="utf-8", errors="replace").strip())
            except Exception:
                pid = 0
            if pid and is_pid_alive(pid):
                return None
            try:
                path.unlink()
            except OSError:
                pass
            continue
        except OSError:
            return None
        try:
            os.write(handle, str(os.getpid()).encode("ascii"))
        except OSError:
            pass
        return handle
    return None


def release_pid_lock(handle: Optional[int], path: Path = PID_FILE) -> None:
    if handle is not None:
        try:
            os.close(handle)
        except OSError:
            pass
    try:
        path.unlink()
    except OSError:
        pass


def watcher_running_pid() -> Optional[int]:
    if not PID_FILE.exists():
        return None
    try:
        pid = int(PID_FILE.read_text(encoding="utf-8", errors="replace").strip())
    except Exception:
        return None
    return pid if is_pid_alive(pid) else None


__all__ = ["acquire_pid_lock", "release_pid_lock", "watcher_running_pid"]
