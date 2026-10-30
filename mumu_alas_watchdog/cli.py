"""命令行入口与各子命令。"""

from __future__ import annotations

import argparse
import os
import sys
import time
from pathlib import Path
from typing import List, Optional

from . import autostart
from .config import Settings
from .constants import (
    APP_NAME,
    CONFIG_PATH,
    FLAG_ALAS_RESTART,
    FLAG_FULL_RESTART,
    FLAG_STOP,
    PID_FILE,
    VERSION,
    WD_DIR,
)
from .layout import LayoutError
from .lockfile import acquire_pid_lock, release_pid_lock, watcher_running_pid
from .util import background_command, kill_pid_tree, now_text
from .win32api import attach_parent_console, clean_launch_env, is_pid_alive, win32_spawn
from .watchdog import Watchdog


def _prepare_stdio() -> None:
    # 窗口化 exe 从命令行调用时附加父控制台
    if sys.stdout is None:
        if attach_parent_console():
            try:
                sys.stdout = open("CONOUT$", "w", encoding="utf-8", errors="replace", buffering=1)
                sys.stderr = sys.stdout
            except OSError:
                pass
    # GUI 调用子命令：把输出写入临时文件
    command_log = os.environ.get("MUW_CMD_LOG")
    if command_log:
        try:
            captured = open(command_log, "w", encoding="utf-8", errors="replace", buffering=1)
            sys.stdout = captured
            sys.stderr = captured
        except OSError:
            pass
    for name in ("stdout", "stderr"):
        stream = getattr(sys, name)
        if stream is None:
            continue
        try:
            if not stream.isatty():
                stream.reconfigure(encoding="utf-8", errors="replace")
        except Exception:
            pass


def _show_gui_error(text: str) -> None:
    try:
        import tkinter as tk
        from tkinter import messagebox

        root = tk.Tk()
        root.withdraw()
        messagebox.showerror(f"{APP_NAME} v{VERSION}", text)
        root.destroy()
    except Exception:
        print(text)


def _load_settings(config_path: Path) -> Optional[Settings]:
    try:
        return Settings(config_path)
    except Exception as exc:
        message = f"读取配置失败: {exc}"
        _show_gui_error(message)
        print(message)
        return None


def _cmd_check(settings: Settings) -> int:
    try:
        watchdog = Watchdog(settings, to_file_log=False)
    except Exception as exc:
        print(f"检查失败: {exc}")
        return 1
    try:
        watchdog.print_status()
    except Exception as exc:
        print(f"状态检查出错: {exc}")
        return 1
    return 0


def _cmd_start(config_path: Path) -> int:
    pid = watcher_running_pid()
    if pid:
        print(f"看门狗已经在运行（PID {pid}），无需重复启动")
        return 0
    command = background_command(config_path)
    if not command:
        print("找不到 pythonw.exe，无法后台启动。可以直接运行: python -m mumu_alas_watchdog --cli")
        return 1
    try:
        child_pid = win32_spawn(
            command[0],
            args=command[1:],
            cwd=WD_DIR,
            env=clean_launch_env(),
            hidden=True,
            detached=True,
        )
        print(f"后台进程已创建（PID {child_pid}）")
    except OSError as exc:
        print(f"Win32 CreateProcessW 启动失败: {exc}")
        return 1
    for _ in range(20):
        time.sleep(0.5)
        pid = watcher_running_pid()
        if pid:
            print(f"看门狗已在后台启动（PID {pid}）")
            try:
                log_dir = Settings(config_path).log_dir
            except Exception:
                log_dir = WD_DIR / "logs"
            print(f"  日志目录: {log_dir}")
            print("  查看状态: --check    停止: --stop")
            return 0
    print("启动后未检测到 watchdog.pid，请运行 --check 或查看日志目录排错")
    return 1


def _cmd_stop() -> int:
    pid = watcher_running_pid()
    if not pid:
        for path in (PID_FILE, FLAG_STOP, FLAG_FULL_RESTART, FLAG_ALAS_RESTART):
            try:
                path.unlink()
            except OSError:
                pass
        print("看门狗未在运行")
        return 0
    try:
        FLAG_STOP.write_text(now_text(), encoding="utf-8")
    except OSError as exc:
        print(f"写入停止请求失败: {exc}")
        return 1
    print(f"已发送停止请求（PID {pid}），等待看门狗退出...")
    deadline = time.time() + 300
    last_report = time.time()
    while time.time() < deadline:
        if not is_pid_alive(pid):
            break
        if time.time() - last_report > 30:
            last_report = time.time()
            print("  看门狗仍在运行（可能正在执行重启流程）...")
        time.sleep(1)
    else:
        print("看门狗 300 秒内未退出，可能卡在重启流程中，强制结束（状态可能不完整）")
        kill_pid_tree(pid)
        time.sleep(1)
    for path in (PID_FILE, FLAG_STOP, FLAG_FULL_RESTART, FLAG_ALAS_RESTART):
        try:
            path.unlink()
        except OSError:
            pass
    print("看门狗已停止")
    return 0


def _cmd_restart_now(settings: Settings) -> int:
    if watcher_running_pid():
        try:
            FLAG_FULL_RESTART.write_text(now_text(), encoding="utf-8")
        except OSError as exc:
            print(f"写入重启请求失败: {exc}")
            return 1
        print("检测到看门狗正在运行，已通知它执行完整重启（详见日志目录）")
        return 0
    watchdog = Watchdog(settings, to_file_log=True)
    return 0 if watchdog.run_full_cycle("命令行手动请求") else 2


def _cmd_restart_alas_now(settings: Settings) -> int:
    if watcher_running_pid():
        try:
            FLAG_ALAS_RESTART.write_text(now_text(), encoding="utf-8")
        except OSError as exc:
            print(f"写入重启请求失败: {exc}")
            return 1
        print("检测到看门狗正在运行，已通知它重启 Alas（不重启模拟器）")
        return 0
    watchdog = Watchdog(settings, to_file_log=True)
    if not watchdog._emulator_quick_ok():
        print("模拟器或 ADB 当前异常，改为执行完整重启")
        return 0 if watchdog.run_full_cycle("命令行手动请求（模拟器/ADB 异常）") else 2
    return 0 if watchdog.restart_alas_only("命令行手动请求") else 2


def _cmd_test_emulator(settings: Settings, index: int, keep_running: bool) -> int:
    import json

    running = watcher_running_pid()
    if running:
        print(f"看门狗正在运行（PID {running}），请先执行 --stop 再测试")
        return 2
    watchdog = Watchdog(settings, to_file_log=True)
    if index == watchdog.layout.instance and watchdog.alas.electron_processes():
        print(
            f"拒绝测试：实例 {index} 正是 Alas 正在使用的模拟器，"
            "测试会中断当前 Alas 任务。请先关闭 Alas，或改成其他实例编号。"
        )
        return 2

    old_instance = watchdog.layout.instance
    old_serial = watchdog.mumu.serial
    old_kill_adb = watchdog.settings.kill_adb_server
    watchdog.settings.kill_adb_server = False  # 测试其他实例时不动共用 adb server
    watchdog.layout.instance = index
    watchdog.mumu.instance = index
    watchdog.mumu.serial = f"127.0.0.1:{16384 + 32 * index}"
    print("=" * 72)
    print(f"模拟器控制实测：实例 {index}（不会动 Alas）")
    print("=" * 72)
    success = False
    try:
        try:
            info = watchdog.mumu.info()
        except Exception:
            info = {}
        if watchdog.mumu.is_running(info):
            print("实例当前已在运行，跳过启动步骤")
        else:
            print("启动实例 ...")
            info = watchdog.mumu.launch()
            print(
                "启动完成: "
                + json.dumps(
                    {key: info.get(key) for key in ("name", "pid", "adb_port", "player_state")},
                    ensure_ascii=False,
                )
            )
        watchdog.mumu.wait_adb_ready()
        info = watchdog.mumu.info()
        print(f"ADB 正常: {watchdog.mumu.serial}")
        state = watchdog.mumu.adb_state()
        boot = watchdog.mumu.adb_shell(["getprop", "sys.boot_completed"], timeout=15)
        print(f"adb state = {state}, sys.boot_completed = {(boot.out or '').strip()}")
        pid = info.get("pid")
        if pid:
            from .win32api import proc_memory

            working_set, private = proc_memory(int(pid))
            print(f"进程内存: 工作集 {working_set / 2 ** 30:.2f} GB / 提交 {private / 2 ** 30:.2f} GB")
        success = True
    except Exception as exc:
        print(f"测试失败: {exc}")
    finally:
        if not keep_running:
            print("关闭测试实例 ...")
            try:
                watchdog.mumu.shutdown()
                print("测试实例已关闭")
            except Exception as exc:
                print(f"关闭测试实例失败: {exc}")
        watchdog.layout.instance = old_instance
        watchdog.mumu.instance = old_instance
        watchdog.mumu.serial = old_serial
        watchdog.settings.kill_adb_server = old_kill_adb
    print("=" * 72)
    print("测试结果:", "通过" if success else "失败（详情见日志目录）")
    return 0 if success else 2


def _cmd_daemon(settings: Settings) -> int:
    for path in (FLAG_STOP, FLAG_FULL_RESTART, FLAG_ALAS_RESTART):
        try:
            path.unlink()
        except OSError:
            pass
    lock_handle = acquire_pid_lock(PID_FILE)
    if lock_handle is None:
        print(f"{APP_NAME} 已经在运行（PID 文件: {PID_FILE}），本次不重复启动")
        return 0
    try:
        watchdog = Watchdog(settings, to_file_log=True)
        watchdog.run()
    except KeyboardInterrupt:
        pass
    except Exception as exc:
        import logging
        import traceback

        logging.getLogger(APP_NAME).error(f"看门狗异常退出: {exc}")
        logging.getLogger(APP_NAME).debug(traceback.format_exc())
        return 1
    finally:
        release_pid_lock(lock_handle, PID_FILE)
    return 0


def _has_cli_action(args) -> bool:
    return bool(
        args.check
        or args.restart_now
        or args.restart_alas_now
        or args.test_emulator is not None
        or args.cli
        or args.daemon
    )


def main(argv: Optional[List[str]] = None) -> int:
    _prepare_stdio()
    parser = argparse.ArgumentParser(
        description="MuMu 模拟器 + Alas 自动重启看门狗（GUI 控制台 + CLI 模式；单文件 exe 分发；不改 Alas 代码）",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=(
            "模式:\n"
            "  无参数                                打开 GUI 控制台\n"
            "  --gui                                 打开 GUI 控制台\n"
            "  --cli                                 前台 CLI 守护模式（控制台日志，调试用）\n"
            "  --start                               后台启动看门狗（无窗口）\n"
            "  --stop                                停止后台看门狗\n"
            "  --check                               查看运行状态（只读）\n"
            "  --restart-now                         立即完整重启（模拟器 + Alas）\n"
            "  --restart-alas-now                    只重启 Alas（不重启模拟器）\n"
            "  --install-autostart / --uninstall-autostart  开机自启管理\n"
            "  --test-emulator 1                     实测未使用实例的启动/关闭\n"
        ),
    )
    parser.add_argument("--config", default=str(CONFIG_PATH), help="配置文件路径")
    parser.add_argument("--gui", action="store_true", help="打开 GUI 控制台（默认）")
    parser.add_argument("--cli", action="store_true", help="前台 CLI 守护模式（控制台日志）")
    parser.add_argument("--daemon", action="store_true", help=argparse.SUPPRESS)
    parser.add_argument("--start", action="store_true", help="后台启动看门狗（无窗口）")
    parser.add_argument("--stop", action="store_true", help="停止正在运行的看门狗")
    parser.add_argument("--check", action="store_true", help="只读检查当前状态")
    parser.add_argument("--restart-now", action="store_true", help="立即执行一次完整重启")
    parser.add_argument("--restart-alas-now", action="store_true", help="只重启 Alas")
    parser.add_argument("--test-emulator", type=int, default=None, metavar="INDEX", help="实测指定 MuMu 实例")
    parser.add_argument("--keep-running", action="store_true", help="配合 --test-emulator：测试后不关闭实例")
    parser.add_argument("--install-autostart", action="store_true", help="安装开机自启")
    parser.add_argument("--uninstall-autostart", action="store_true", help="移除开机自启")
    parser.add_argument("--version", action="store_true", help="显示版本号")
    args = parser.parse_args(argv)

    if args.version:
        print(f"{APP_NAME} v{VERSION}")
        return 0

    config_path = Path(args.config).resolve()

    try:
        if args.uninstall_autostart:
            return autostart.uninstall()
        if args.install_autostart:
            return autostart.install()
        if args.stop:
            return _cmd_stop()
        if args.start:
            return _cmd_start(config_path)
    except KeyboardInterrupt:
        print("已取消")
        return 130

    settings = _load_settings(config_path)
    if settings is None:
        return 2
    if settings.created:
        print(f"已生成默认配置文件: {config_path}")

    try:
        if args.check:
            return _cmd_check(settings)
        if args.restart_now:
            return _cmd_restart_now(settings)
        if args.restart_alas_now:
            return _cmd_restart_alas_now(settings)
        if args.test_emulator is not None:
            return _cmd_test_emulator(settings, args.test_emulator, args.keep_running)
        if args.cli or args.daemon:
            return _cmd_daemon(settings)
        from .gui import run_gui

        try:
            return run_gui(settings)
        except Exception as exc:
            _show_gui_error(
                f"GUI 启动失败：{exc}\n\n可以改用命令行模式：python -m mumu_alas_watchdog --cli"
            )
            return 1
    except KeyboardInterrupt:
        print("已取消")
        return 130
    except Exception as exc:
        print(f"错误: {exc}")
        return 2


__all__ = ["main"]
