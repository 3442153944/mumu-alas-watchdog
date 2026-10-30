"""开机自启（当前用户 Startup 目录快捷方式）。"""

from __future__ import annotations

import os
from pathlib import Path
from typing import Optional

from .constants import APP_NAME, APP_PATH, AUTOSTART_SHORTCUT_NAME, FROZEN, WD_DIR
from .util import ps_quote, pythonw_path, run_cmd


def startup_shortcut_path() -> Optional[Path]:
    appdata = os.environ.get("APPDATA")
    if not appdata:
        return None
    return (
        Path(appdata)
        / "Microsoft"
        / "Windows"
        / "Start Menu"
        / "Programs"
        / "Startup"
        / AUTOSTART_SHORTCUT_NAME
    )


def is_installed() -> bool:
    path = startup_shortcut_path()
    return bool(path and path.exists())


def install() -> int:
    shortcut = startup_shortcut_path()
    if shortcut is None:
        print("找不到当前用户的 Startup 目录（APPDATA 环境变量缺失）")
        return 1
    if FROZEN:
        target = str(APP_PATH)
        arguments = "--start"
    else:
        pythonw = pythonw_path()
        if not pythonw:
            print("找不到 pythonw.exe，无法安装开机自启。请确认 Python 安装完整。")
            return 1
        target = pythonw
        arguments = "-m mumu_alas_watchdog --start"
    script = (
        "$ws = New-Object -ComObject WScript.Shell;"
        f"$lnk = $ws.CreateShortcut({ps_quote(str(shortcut))});"
        f"$lnk.TargetPath = {ps_quote(target)};"
        f"$lnk.Arguments = {ps_quote(arguments)};"
        f"$lnk.WorkingDirectory = {ps_quote(str(WD_DIR))};"
        "$lnk.WindowStyle = 7;"
        f"$lnk.Description = {ps_quote(APP_NAME + ' autostart')};"
        "$lnk.Save()"
    )
    result = run_cmd(
        ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script],
        timeout=60,
    )
    if result.timed_out or result.rc != 0 or not shortcut.exists():
        print("创建开机自启快捷方式失败:")
        print((result.out or "").strip())
        print((result.err or "").strip())
        return 1
    print(f"已安装开机自启: {shortcut}")
    print("下次登录 Windows 会自动后台启动看门狗；现在启动请运行: --start")
    return 0


def uninstall() -> int:
    shortcut = startup_shortcut_path()
    if shortcut is None or not shortcut.exists():
        print("未找到开机自启快捷方式（可能本来就未安装）")
        return 0
    try:
        shortcut.unlink()
    except OSError as exc:
        print(f"删除开机自启快捷方式失败: {exc}")
        return 1
    print(f"已移除开机自启: {shortcut}")
    return 0


__all__ = ["startup_shortcut_path", "is_installed", "install", "uninstall"]
