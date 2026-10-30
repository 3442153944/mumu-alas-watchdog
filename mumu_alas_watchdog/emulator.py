"""MuMu 模拟器控制：MuMuManager CLI 封装、启停、ADB 连接等待。"""

from __future__ import annotations

import json
import time
from pathlib import Path
from typing import Dict, Optional, Tuple

from .config import Settings
from .layout import Layout
from .util import RunResult, run_cmd
from .win32api import is_pid_alive, list_processes, proc_exe


class MuMuError(RuntimeError):
    pass


class MuMuClient:
    def __init__(self, settings: Settings, layout: Layout, logger):
        self.settings = settings
        self.layout = layout
        self.logger = logger
        self.manager: Path = layout.mumu_manager
        self.adb_exe: Path = layout.adb_exe
        self.instance: int = layout.instance
        self.serial: str = layout.serial
        self.last_pid: Optional[int] = None

    # ------------------------------------------------------------------
    # MuMuManager CLI
    # ------------------------------------------------------------------
    def raw(self, args, timeout: Optional[float] = None):
        command = [self.manager] + [str(item) for item in args]
        result = run_cmd(command, timeout=timeout or self.settings.mumu_cmd_timeout)
        text = (result.out or "").strip() or (result.err or "").strip()
        if result.timed_out:
            raise MuMuError(
                f"MuMuManager 命令超时 {int(timeout or self.settings.mumu_cmd_timeout)} 秒: "
                f"{' '.join(str(a) for a in args)}"
            )
        data = None
        if text.startswith("{"):
            try:
                data = json.loads(text)
            except Exception:
                data = None
        if isinstance(data, dict):
            code = data.get("errcode")
            if code is None:
                code = data.get("error_code")
            if code not in (0, None):
                raise MuMuError(f"MuMuManager 返回错误: {data}")
            return data
        if result.rc != 0:
            raise MuMuError(f"MuMuManager 命令失败 rc={result.rc}: {text[:300]}")
        return text

    def info(self, index: Optional[int] = None, timeout: float = 25) -> Dict:
        idx = self.instance if index is None else int(index)
        data = self.raw(["info", "-v", str(idx)], timeout=timeout)
        if not isinstance(data, dict) or "index" not in data:
            raise MuMuError(f"无法解析 MuMu 实例 {idx} 的信息: {data!r}")
        return data

    def info_all(self, timeout: float = 30) -> Dict:
        data = self.raw(["info", "-v", "all"], timeout=timeout)
        return data if isinstance(data, dict) else {}

    @staticmethod
    def is_running(info: Dict) -> bool:
        return bool(info.get("is_process_started")) and bool(info.get("is_android_started"))

    def refresh(self) -> Dict:
        """查询实例信息，并同步 serial / pid / 启动时间。"""
        info = self.info()
        pid = info.get("pid")
        try:
            self.last_pid = int(pid) if pid else None
        except (TypeError, ValueError):
            self.last_pid = None
        port = info.get("adb_port")
        if port:
            host = info.get("adb_host_ip") or "127.0.0.1"
            self.serial = f"{host}:{int(port)}"
            self.layout.serial = self.serial
        return info

    # ------------------------------------------------------------------
    # ADB
    # ------------------------------------------------------------------
    def adb(self, args, timeout: float = 20) -> RunResult:
        return run_cmd([self.adb_exe] + [str(item) for item in args], timeout=timeout)

    def adb_state(self) -> str:
        result = self.adb(["-s", self.serial, "get-state"], timeout=15)
        text = (result.out or "").strip() or (result.err or "").strip()
        return text.strip().lower()

    def timed_adb_state(self) -> Tuple[str, float]:
        started = time.time()
        state = self.adb_state()
        return state, (time.time() - started) * 1000

    def adb_shell(self, args, timeout: float = 20) -> RunResult:
        return self.adb(["-s", self.serial, "shell"] + [str(item) for item in args], timeout=timeout)

    def kill_adb_server(self) -> None:
        result = self.adb(["kill-server"], timeout=30)
        if result.timed_out or result.rc != 0:
            for process in list_processes():
                if process.name.lower() != self.adb_exe.name.lower():
                    continue
                executable = proc_exe(process.pid)
                if executable and Path(executable).resolve() == self.adb_exe.resolve():
                    run_cmd(["taskkill", "/F", "/PID", str(process.pid)], timeout=30)
            time.sleep(1)

    def _wait_device(self, timeout: float) -> bool:
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                state = self.adb_state()
                if state == "device":
                    return True
                if "unauthorized" in state:
                    self.logger.warning("[ADB] 设备未授权，请检查模拟器调试授权弹窗")
            except Exception:
                pass
            time.sleep(2)
        return False

    def _wait_boot(self, timeout: float) -> bool:
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                result = self.adb_shell(["getprop", "sys.boot_completed"], timeout=15)
                if (result.out or "").strip() == "1":
                    return True
            except Exception:
                pass
            time.sleep(2)
        return False

    def wait_adb_ready(self) -> None:
        try:
            self.refresh()
        except Exception:
            pass
        self.logger.info(f"[ADB] 等待连接恢复: {self.serial}")
        if self.settings.kill_adb_server:
            self.logger.info("[ADB] 结束旧 adb server 以清除卡死连接")
            self.kill_adb_server()
        self.adb(["start-server"], timeout=60)

        deadline = time.time() + self.settings.adb_ready_timeout
        attempt = 0
        last_report = 0.0
        while time.time() < deadline:
            attempt += 1
            self.adb(["connect", self.serial], timeout=30)
            if self._wait_device(30) and self._wait_boot(120):
                result = self.adb_shell(["echo", "watchdog-ok"], timeout=15)
                if "watchdog-ok" in (result.out or ""):
                    self.logger.info(f"[ADB] 连接已恢复（尝试 {attempt} 次）")
                    return
            try:
                self.adb(["disconnect", self.serial], timeout=15)
            except Exception:
                pass
            time.sleep(3)
            if time.time() - last_report > 30:
                last_report = time.time()
                self.logger.info("[ADB] 尚未就绪，继续等待...")

        self.logger.warning("[ADB] 常规连接失败，尝试通过 MuMuManager 连接")
        try:
            data = self.raw(["adb", "-v", str(self.instance), "-c", "connect"], timeout=60)
            self.logger.info(f"[ADB] MuMuManager connect 返回: {data}")
            if self._wait_device(30) and self._wait_boot(60):
                return
        except Exception as exc:
            self.logger.warning(f"[ADB] MuMuManager connect 失败: {exc}")
        raise TimeoutError(
            f"ADB 连接恢复超时（{self.settings.adb_ready_timeout} 秒），当前设备 {self.serial}"
        )

    # ------------------------------------------------------------------
    # MuMu 主程序恢复
    # ------------------------------------------------------------------
    def ensure_manager(self) -> None:
        try:
            self.info()
            return
        except Exception as exc:
            self.logger.warning(f"[MuMu] 控制接口无响应: {exc}")

        self.logger.info("[MuMu] 尝试启动/恢复 MuMu 主程序...")
        try:
            self.raw(["main", "launch"], timeout=60)
        except Exception:
            pass
        if self._wait_manager_ready(45):
            return

        self.logger.warning("[MuMu] 主程序可能卡死，重启 MuMu 主程序（会关闭当前所有 MuMu 实例）")
        for process in list_processes():
            if process.name.lower() == "mumunxmain.exe":
                run_cmd(["taskkill", "/F", "/T", "/PID", str(process.pid)], timeout=30)
        time.sleep(2)
        main_exe = self.manager.parent / "MuMuNxMain.exe"
        if not main_exe.exists():
            raise MuMuError(f"找不到 MuMu 主程序 {main_exe}，请手动重启 MuMu 模拟器")
        from .win32api import win32_spawn

        try:
            win32_spawn(main_exe, cwd=main_exe.parent, hidden=True)
        except OSError as exc:
            raise MuMuError(f"启动 MuMu 主程序失败: {exc}")
        if not self._wait_manager_ready(90):
            raise MuMuError("MuMu 控制接口无法恢复，请手动重启 MuMu 模拟器后再试")

    def _wait_manager_ready(self, timeout: float) -> bool:
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                if self.info():
                    self.logger.info("[MuMu] 控制接口已恢复")
                    return True
            except Exception:
                pass
            time.sleep(3)
        return False

    # ------------------------------------------------------------------
    # 启停
    # ------------------------------------------------------------------
    def _wait_stopped(self, timeout: float, fallback_pid: Optional[int]) -> bool:
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                if not self.info().get("is_process_started"):
                    return True
            except Exception:
                if fallback_pid and not is_pid_alive(fallback_pid):
                    return True
            time.sleep(2)
        return False

    def shutdown(self) -> None:
        try:
            info = self.info()
        except Exception:
            self.ensure_manager()
            info = self.info()

        if not info.get("is_process_started"):
            self.logger.info("[MuMu] 实例已是关闭状态")
            return
        pid = info.get("pid")
        try:
            pid = int(pid) if pid else None
        except (TypeError, ValueError):
            pid = None
        self.last_pid = pid
        self.logger.info(f"[MuMu] 关闭实例 {self.instance}（{info.get('name') or ''}）...")
        try:
            self.raw(["control", "-v", str(self.instance), "shutdown"], timeout=120)
        except Exception as exc:
            self.logger.warning(f"[MuMu] 发送关闭命令失败: {exc}")

        if self._wait_stopped(self.settings.emulator_shutdown_timeout, pid):
            self.logger.info("[MuMu] 实例已关闭")
            time.sleep(3)
            return

        self.logger.warning("[MuMu] 未能正常关闭，尝试强制结束实例进程")
        if pid:
            run_cmd(["taskkill", "/F", "/T", "/PID", str(pid)], timeout=30)
        if self._wait_stopped(30, pid):
            self.logger.info("[MuMu] 实例已被强制关闭")
            time.sleep(2)
            return

        self.logger.warning("[MuMu] 仍然无法关闭，最后尝试重启 MuMu 主程序")
        for process in list_processes():
            if process.name.lower() == "mumunxmain.exe":
                run_cmd(["taskkill", "/F", "/T", "/PID", str(process.pid)], timeout=30)
        time.sleep(3)
        for process in list_processes():
            if process.name.lower() == "mumunxdevice.exe":
                run_cmd(["taskkill", "/F", "/T", "/PID", str(process.pid)], timeout=30)
        time.sleep(2)

    def wait_started(self, timeout: float) -> Dict:
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                info = self.info()
                if self.is_running(info):
                    state = info.get("player_state")
                    if state in (None, "", "start_finished"):
                        return info
                error_code = info.get("launch_err_code")
                if error_code not in (0, None):
                    self.logger.warning(
                        f"[MuMu] 启动报错 {error_code}: {info.get('launch_err_msg')}"
                    )
            except Exception as exc:
                self.logger.debug(f"[MuMu] 查询状态失败: {exc}")
            time.sleep(3)
        raise TimeoutError(f"等待 MuMu 实例启动超时（{int(timeout)} 秒）")

    def launch(self) -> Dict:
        self.ensure_manager()
        for attempt in range(1, 4):
            try:
                info = self.info()
                if self.is_running(info):
                    self.logger.info("[MuMu] 实例已在运行")
                    self.refresh()
                    return info
            except Exception as exc:
                self.logger.warning(f"[MuMu] 查询状态失败: {exc}")

            self.logger.info(f"[MuMu] 启动实例 {self.instance}（第 {attempt} 次）...")
            try:
                self.raw(["control", "-v", str(self.instance), "launch"], timeout=120)
            except Exception as exc:
                self.logger.warning(f"[MuMu] 启动命令失败: {exc}")

            try:
                info = self.wait_started(self.settings.emulator_boot_timeout)
                self.refresh()
                self.logger.info(
                    f"[MuMu] 实例启动完成，pid={info.get('pid')}，adb={self.serial}"
                )
                time.sleep(5)
                return info
            except TimeoutError as exc:
                self.logger.warning(f"[MuMu] {exc}")
                try:
                    self.raw(["control", "-v", str(self.instance), "shutdown"], timeout=60)
                except Exception:
                    pass
                time.sleep(5)
        raise MuMuError("MuMu 实例多次启动失败")

    def emulator_start_time(self):
        from .win32api import proc_create_time

        if not self.last_pid:
            return None
        return proc_create_time(self.last_pid)


__all__ = ["MuMuClient", "MuMuError"]
