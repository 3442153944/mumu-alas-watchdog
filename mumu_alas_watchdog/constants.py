"""全局常量与运行路径（源码运行 / PyInstaller onefile 均适用）。"""

from __future__ import annotations

import sys
from pathlib import Path

APP_NAME = "MuMuAlasWatchdog"
VERSION = "1.3.0"
FROZEN = bool(getattr(sys, "frozen", False))

if FROZEN:
    WD_DIR = Path(sys.executable).resolve().parent
    APP_PATH = Path(sys.executable).resolve()
else:
    # 包目录的上一级 = 项目根目录（config.yml / logs 所在位置）
    WD_DIR = Path(__file__).resolve().parent.parent
    APP_PATH = WD_DIR / "main.py"

CONFIG_PATH = WD_DIR / "config.yml"
PID_FILE = WD_DIR / "watchdog.pid"
STATUS_JSON = WD_DIR / "status.json"
FLAG_FULL_RESTART = WD_DIR / "restart_now.flag"
FLAG_ALAS_RESTART = WD_DIR / "restart_alas_now.flag"
FLAG_STOP = WD_DIR / "stop.flag"
DEFAULT_LOG_DIR = WD_DIR / "logs"
AUTOSTART_SHORTCUT_NAME = "MuMuAlasWatchdog.lnk"

CREATE_NO_WINDOW = 0x08000000
DETACHED_PROCESS = 0x00000008

# Alas 日志里出现这些字样说明需要人工处理，仅告警，不自动重启
FATAL_LOG_PATTERNS = (
    "Request human takeover",
    "RequestHumanTakeover",
    "GamePageUnknownError",
    "failed 3 or more times",
    "crashed",
    "ScriptError",
)

__all__ = [
    "APP_NAME",
    "VERSION",
    "FROZEN",
    "WD_DIR",
    "APP_PATH",
    "CONFIG_PATH",
    "PID_FILE",
    "STATUS_JSON",
    "FLAG_FULL_RESTART",
    "FLAG_ALAS_RESTART",
    "FLAG_STOP",
    "DEFAULT_LOG_DIR",
    "AUTOSTART_SHORTCUT_NAME",
    "CREATE_NO_WINDOW",
    "DETACHED_PROCESS",
    "FATAL_LOG_PATTERNS",
]
