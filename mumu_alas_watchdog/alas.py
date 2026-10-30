"""Alas 控制：进程识别/结束、Win32 启动、WebUI 就绪检测、reloadalas 自动开始脚本。"""

from __future__ import annotations

import time
from pathlib import Path
from typing import List, Optional

from .config import Settings
from .layout import Layout
from .util import http_probe, kill_pid_tree, path_is_under, port_listening, run_cmd, same_path
from .win32api import (
    ProcessInfo,
    clean_launch_env,
    is_pid_alive,
    list_processes,
    proc_exe,
    win32_spawn,
)


class AlasError(RuntimeError):
    pass


class AlasClient:
    def __init__(self, settings: Settings, layout: Layout, logger):
        self.settings = settings
        self.layout = layout
        self.logger = logger

    # ------------------------------------------------------------------
    # 进程识别
    # ------------------------------------------------------------------
    def electron_processes(self) -> List[ProcessInfo]:
        electron_dir = self.layout.alas_root / "toolkit" / "webapp"
        result: List[ProcessInfo] = []
        for process in list_processes():
            if process.name.lower() != "alas.exe":
                continue
            executable = proc_exe(process.pid)
            if executable and path_is_under(executable, electron_dir):
                process.exe = executable
                result.append(process)
        return result

    def electron_main(self) -> Optional[ProcessInfo]:
        processes = self.electron_processes()
        if not processes:
            return None
        pids = {process.pid for process in processes}
        mains = [process for process in processes if process.ppid not in pids]
        return mains[0] if mains else processes[0]

    def electron_main_pid(self) -> Optional[int]:
        main = self.electron_main()
        return main.pid if main else None

    def alas_processes(self) -> List[ProcessInfo]:
        electron_dir = self.layout.alas_root / "toolkit" / "webapp"
        launcher = self.layout.launcher_exe
        toolkit_python = self.layout.toolkit_python
        result: List[ProcessInfo] = []
        for process in list_processes():
            name = process.name.lower()
            if name == "alas.exe":
                executable = proc_exe(process.pid)
                if not executable:
                    continue
                if path_is_under(executable, electron_dir) or same_path(executable, launcher):
                    process.exe = executable
                    result.append(process)
            elif name in ("python.exe", "pythonw.exe"):
                executable = proc_exe(process.pid)
                if executable and same_path(executable, toolkit_python):
                    process.exe = executable
                    result.append(process)
        return result

    # ------------------------------------------------------------------
    # WebUI
    # ------------------------------------------------------------------
    def webui_status(self):
        return http_probe(self.layout.webui_port)

    def webui_ok(self) -> bool:
        return self.webui_status()[0]

    # ------------------------------------------------------------------
    # 结束 Alas
    # ------------------------------------------------------------------
    def find_port_owner(self, port: int) -> Optional[int]:
        result = run_cmd(["netstat", "-ano", "-p", "TCP"], timeout=20)
        for line in (result.out or "").splitlines():
            if f":{port}" not in line or "LISTENING" not in line.upper():
                continue
            parts = line.split()
            if not parts:
                continue
            try:
                return int(parts[-1])
            except ValueError:
                continue
        return None

    def ensure_webui_port_free(self) -> None:
        port = self.layout.webui_port
        if not port_listening(port):
            return
        owner = self.find_port_owner(port)
        if owner:
            self.logger.warning(f"[Alas] WebUI 端口 {port} 仍被 PID {owner} 占用，强制结束")
            kill_pid_tree(owner)
        for _ in range(20):
            if not port_listening(port):
                return
            time.sleep(0.5)
        self.logger.warning(f"[Alas] WebUI 端口 {port} 可能仍被占用，继续尝试启动")

    def stop(self) -> None:
        processes = self.alas_processes()
        if not processes:
            self.logger.info("[Alas] 当前未在运行")
            self.ensure_webui_port_free()
            return
        self.logger.info(f"[Alas] 结束 Alas 进程（{len(processes)} 个）...")
        electron = self.electron_processes()
        pids = {process.pid for process in electron}
        mains = [process for process in electron if process.ppid not in pids]
        killed = set()
        for process in mains:
            kill_pid_tree(process.pid)
            killed.add(process.pid)
        time.sleep(1.5)
        deadline = time.time() + self.settings.kill_alas_timeout
        while time.time() < deadline:
            remaining = self.alas_processes()
            if not remaining:
                break
            for process in remaining:
                if process.pid in killed:
                    continue
                kill_pid_tree(process.pid)
                killed.add(process.pid)
            time.sleep(1.0)
        remaining = self.alas_processes()
        if remaining:
            self.logger.warning(f"[Alas] 仍有进程未退出: {[p.pid for p in remaining]}")
        self.ensure_webui_port_free()
        self.logger.info("[Alas] 已结束")

    # ------------------------------------------------------------------
    # 启动 Alas
    # ------------------------------------------------------------------
    def write_reloadalas(self) -> None:
        if not self.settings.use_reloadalas:
            return
        path = self.layout.alas_root / "config" / "reloadalas"
        try:
            path.write_text("\n".join(self.layout.configs) + "\n", encoding="utf-8")
            self.logger.info(f"[Alas] 已写入 {path}（启动后自动开始游戏脚本）")
        except OSError as exc:
            self.logger.warning(f"[Alas] 写入 reloadalas 失败: {exc}")

    def start(self) -> int:
        executable = self.layout.launch_exe
        if not executable.exists():
            raise AlasError(f"找不到 Alas 启动文件: {executable}")
        self.logger.info(
            f"[Alas] 通过 Win32 CreateProcessW 启动 Alas（{self.layout.launch_mode}）: {executable}"
        )
        try:
            pid = win32_spawn(
                executable,
                cwd=self.layout.alas_root,
                env=clean_launch_env(),
                hidden=True,
            )
        except OSError as exc:
            raise AlasError(f"Win32 启动 Alas 失败: {exc}")
        self.logger.info(f"[Alas] 已创建进程，PID={pid}")
        return pid

    def wait_webui_ready(self, pid: Optional[int]) -> None:
        self.logger.info("[Alas] 等待 WebUI 启动...")
        deadline = time.time() + self.settings.alas_start_timeout
        main_seen = False
        logged_exit = False
        while time.time() < deadline:
            if self.electron_main() is not None:
                main_seen = True
            elif pid and not is_pid_alive(pid) and not main_seen and not logged_exit:
                self.logger.warning("[Alas] Win32 启动的进程已退出，继续等待 Electron 主进程出现")
                logged_exit = True
            if main_seen and self.webui_ok():
                self.logger.info(f"[Alas] WebUI 已就绪（端口 {self.layout.webui_port}）")
                return
            time.sleep(2)
        if pid and not is_pid_alive(pid) and not main_seen:
            raise AlasError("Alas 启动进程已退出，且未检测到 Electron 主进程")
        raise TimeoutError(f"等待 Alas WebUI 启动超时（{self.settings.alas_start_timeout} 秒）")

    def bring_up(self) -> None:
        self.write_reloadalas()
        pid = self.start()
        self.wait_webui_ready(pid)


__all__ = ["AlasClient", "AlasError"]
