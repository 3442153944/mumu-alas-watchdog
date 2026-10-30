"""看门狗主编排：重启流程、健康巡检、定时调度、守护循环、状态输出。"""

from __future__ import annotations

import time
import traceback
from datetime import datetime, timedelta
from pathlib import Path
from typing import List, Optional

from .alas import AlasClient
from .config import Settings
from .constants import (
    APP_NAME,
    FLAG_ALAS_RESTART,
    FLAG_FULL_RESTART,
    FLAG_STOP,
    FROZEN,
    STATUS_JSON,
    VERSION,
)
from .emulator import MuMuClient
from .layout import Layout, detect_layout
from .logging_setup import prune_logs, setup_logging
from .logwatch import LogWatcher
from .schedule import apply_startup_grace, compute_next_restart
from .status import build_status, format_status_lines, write_status
from .util import fmt_duration, now_text
from .win32api import proc_create_time


class Watchdog:
    def __init__(self, settings: Settings, to_file_log: bool = True, console: bool = True):
        self.settings = settings
        self.logger = setup_logging(settings.log_dir, settings.log_level, to_file_log, console=console)
        if to_file_log:
            prune_logs(settings.log_dir, settings.keep_days)

        self.layout: Layout = detect_layout(settings)
        self.mumu = MuMuClient(settings, self.layout, self.logger)
        self.alas = AlasClient(settings, self.layout, self.logger)
        self.logwatch = LogWatcher(settings, self.layout, self.logger)

        self.emulator_start_time: Optional[datetime] = None
        self.refresh_emulator_runtime()

        self.pending_full_cycle: Optional[str] = None
        self.pending_alas_restart: Optional[str] = None
        self.next_restart: Optional[datetime] = None
        self.retry_at: Optional[datetime] = None
        self.next_health: Optional[datetime] = None
        self.next_status_log: Optional[datetime] = None
        self.next_alas_restart_at: datetime = datetime.min
        self.next_stuck_trigger_at: datetime = datetime.min
        self.failures = 0
        self.adb_fail = 0
        self.emulator_missing = 0
        self.mumu_fail = 0
        self.alas_missing = 0
        self.stop_requested = False
        self.cycle_count = 0
        self.last_cycle: Optional[dict] = None
        self.started_at = datetime.now()
        self._stuck_keys = set()

    # ------------------------------------------------------------------
    # 模拟器状态辅助
    # ------------------------------------------------------------------
    def refresh_emulator_runtime(self) -> None:
        try:
            info = self.mumu.info()
            if info.get("is_process_started"):
                self._remember_emulator(info)
        except Exception:
            pass

    def _remember_emulator(self, info: dict) -> None:
        pid = info.get("pid")
        try:
            pid_int = int(pid) if pid else None
        except (TypeError, ValueError):
            pid_int = None
        if pid_int:
            self.mumu.last_pid = pid_int
            created = proc_create_time(pid_int)
            if created:
                self.emulator_start_time = created
        if info.get("name"):
            self.layout.instance_name = info.get("name")

    def _emulator_quick_ok(self) -> bool:
        try:
            info = self.mumu.info()
            if not self.mumu.is_running(info):
                return False
            return self.mumu.adb_state() == "device"
        except Exception:
            return False

    # ------------------------------------------------------------------
    # 重启流程
    # ------------------------------------------------------------------
    def run_full_cycle(self, reason: str) -> bool:
        self.logger.info("=" * 70)
        self.logger.info(f"[重启] 开始完整重启（原因：{reason}）")
        started = time.time()
        snapshot = dict(self.logwatch.offsets)
        success = False
        try:
            self.logger.info("[重启] 步骤 1/6：结束 Alas")
            self.alas.stop()
            self.logger.info("[重启] 步骤 2/6：关闭 MuMu 实例")
            self.mumu.shutdown()
            self.logger.info("[重启] 步骤 3/6：启动 MuMu 实例")
            info = self.mumu.launch()
            self._remember_emulator(info)
            self.logger.info("[重启] 步骤 4/6：等待 ADB 恢复")
            self.mumu.wait_adb_ready()
            self.logger.info("[重启] 步骤 5/6：启动 Alas")
            self.alas.bring_up()
            self.logger.info("[重启] 步骤 6/6：等待游戏脚本自动开始")
            self.logwatch.wait_scheduler_started(
                snapshot,
                electron_alive=lambda: self.alas.electron_main() is not None,
                webui_ok=self.alas.webui_ok,
            )
            self.logwatch.offsets = snapshot
            self.failures = 0
            self.cycle_count += 1
            self.logwatch.manual_stopped = False
            success = True
            self.last_cycle = {
                "time": now_text(),
                "ok": True,
                "reason": reason,
                "duration_seconds": int(time.time() - started),
            }
            self.logger.info(f"[重启] 完整重启成功，用时 {fmt_duration(time.time() - started)}")
        except Exception as exc:
            self.logger.error(f"[重启] 失败: {exc}")
            self.logger.debug(traceback.format_exc())
            self.last_cycle = {
                "time": now_text(),
                "ok": False,
                "reason": reason,
                "duration_seconds": int(time.time() - started),
            }
        finally:
            self._log_emulator_memory("重启后")
            self.log_status_snapshot()
            self.logger.info("=" * 70)
        return success

    def restart_alas_only(self, reason: str) -> bool:
        self.logger.info("=" * 70)
        self.logger.info(f"[重启] 开始重启 Alas（原因：{reason}，不重启模拟器）")
        started = time.time()
        snapshot = dict(self.logwatch.offsets)
        try:
            self.alas.stop()
            self.alas.bring_up()
            self.logwatch.wait_scheduler_started(
                snapshot,
                electron_alive=lambda: self.alas.electron_main() is not None,
                webui_ok=self.alas.webui_ok,
            )
            self.logwatch.offsets = snapshot
            self.failures = 0
            self.logwatch.manual_stopped = False
            self.logger.info(f"[重启] Alas 重启成功，用时 {fmt_duration(time.time() - started)}")
            self.log_status_snapshot()
            return True
        except Exception as exc:
            self.logger.error(f"[重启] Alas 重启失败: {exc}")
            self.logger.debug(traceback.format_exc())
            self.logger.info("=" * 70)
            return False

    def ensure_running_on_start(self) -> None:
        self.logger.info("[启动] 检查初始运行环境...")
        try:
            info = self.mumu.info()
        except Exception:
            self.mumu.ensure_manager()
            info = self.mumu.info()

        if not self.mumu.is_running(info):
            self.logger.info("[启动] 模拟器未运行，正在启动")
            info = self.mumu.launch()
        self._remember_emulator(info)

        try:
            state = self.mumu.adb_state()
        except Exception:
            state = ""
        if state != "device":
            self.logger.info(f"[启动] ADB 状态异常（{state or '无响应'}），尝试恢复连接")
            try:
                self.mumu.wait_adb_ready()
            except Exception as exc:
                self.logger.error(f"[启动] ADB 恢复失败: {exc}")

        main = self.alas.electron_main()
        webui_ok, _ = self.alas.webui_status()
        if main:
            # 只要 Electron 主进程在，就认为 Alas 已运行；WebUI 短暂不可访问交给后续巡检处理，
            # 避免启动时因为一次探测失败就杀掉并重启整个 Alas。
            self.logger.info(f"[启动] Alas 已在运行（Electron PID {main.pid}）")
            if not webui_ok:
                self.logger.warning("[启动] WebUI 暂时不可访问，交由巡检继续处理")
            return

        self.logger.info("[启动] Alas 未运行，正在启动")
        snapshot = dict(self.logwatch.offsets)
        self.alas.stop()
        self.alas.bring_up()
        self.logwatch.wait_scheduler_started(
            snapshot,
            electron_alive=lambda: self.alas.electron_main() is not None,
            webui_ok=self.alas.webui_ok,
        )
        self.logwatch.offsets = snapshot

    # ------------------------------------------------------------------
    # 计划 / 巡检
    # ------------------------------------------------------------------
    def compute_next_restart(self, now: datetime) -> datetime:
        return compute_next_restart(
            now,
            self.settings.restart_times,
            self.settings.restart_interval_hours,
            self.emulator_start_time,
        )

    def health_check(self) -> None:
        texts = self.logwatch.scan()
        self.logwatch.analyze(texts)

        try:
            info = self.mumu.info()
            self.mumu_fail = 0
        except Exception as exc:
            self.mumu_fail += 1
            self.logger.warning(
                f"[巡检] MuMu 信息查询失败（{self.mumu_fail}/{self.settings.adb_fail_threshold}）: {exc}"
            )
            if self.mumu_fail >= self.settings.adb_fail_threshold and self.settings.restart_if_emulator_down:
                self.pending_full_cycle = "MuMu 控制接口无响应"
            return

        if not self.mumu.is_running(info):
            self.emulator_missing += 1
            self.logger.warning(f"[巡检] 检测到 MuMu 实例未运行（{self.emulator_missing}/2）")
            if self.emulator_missing >= 2 and self.settings.restart_if_emulator_down:
                self.pending_full_cycle = "模拟器未运行"
            return
        self.emulator_missing = 0
        self._remember_emulator(info)

        limit = self.settings.mumu_memory_limit_gb
        if limit > 0:
            from .win32api import proc_memory

            pid = info.get("pid")
            if pid:
                _, private = proc_memory(int(pid))
                used_gb = private / 2 ** 30
                if used_gb > limit:
                    self.logger.warning(
                        f"[巡检] 模拟器提交内存 {used_gb:.2f} GB 超过阈值 {limit} GB，触发提前重启"
                    )
                    self.pending_full_cycle = f"模拟器内存超限（{used_gb:.2f} GB）"
                    return

        state, adb_ms = self.mumu.timed_adb_state()
        if state != "device":
            self.adb_fail += 1
            self.logger.warning(
                f"[巡检] ADB 连接异常（{self.adb_fail}/{self.settings.adb_fail_threshold}），"
                f"当前: {state or '无响应'}（{int(adb_ms)} ms）"
            )
            try:
                self.mumu.adb(["connect", self.mumu.serial], timeout=30)
            except Exception:
                pass
            if self.adb_fail >= self.settings.adb_fail_threshold:
                self.pending_full_cycle = "ADB 连接丢失"
            return
        self.adb_fail = 0

        self.check_alas_alive()
        self.check_task_stuck()

    def check_alas_alive(self) -> None:
        main = self.alas.electron_main()
        webui_ok, webui_ms = self.alas.webui_status()
        if main is not None and webui_ok:
            self.alas_missing = 0
            self.logwatch.manual_warned = False
            return
        self.alas_missing += 1
        self.logger.warning(
            f"[巡检] Alas 未正常运行（第 {self.alas_missing} 次："
            f"Electron={'在' if main else '无'}，WebUI={'正常' if webui_ok else '异常'}"
            f"，响应 {int(webui_ms)} ms）"
        )
        if self.alas_missing < 2:
            return
        if self.logwatch.manual_stopped:
            if not self.logwatch.manual_warned:
                self.logger.info("[巡检] Alas 处于手动停止状态，跳过自动恢复（--restart-alas-now 可立即恢复）")
                self.logwatch.manual_warned = True
            return
        if not self.settings.restart_alas_if_closed:
            return
        if datetime.now() < self.next_alas_restart_at:
            return
        self.pending_alas_restart = "Alas 进程/WebUI 丢失"
        self.next_alas_restart_at = datetime.now() + timedelta(seconds=self.settings.alas_restart_cooldown)

    def check_task_stuck(self) -> None:
        if self.settings.task_stuck_minutes <= 0:
            return
        now = datetime.now()
        for config_name, state in self.logwatch.task_state.items():
            task = state.current_task
            if not task:
                continue
            last = state.last_activity or state.task_started
            if not last:
                continue
            idle_minutes = (now - last).total_seconds() / 60
            if idle_minutes < self.settings.task_stuck_minutes:
                continue
            key = (config_name, task, state.task_started)
            if key not in self._stuck_keys:
                self._stuck_keys.add(key)
                self.logger.warning(
                    f"[巡检] 任务 {task} 已 {idle_minutes:.0f} 分钟没有任何日志输出，疑似卡死"
                )
            if self.settings.restart_on_task_stuck and now >= self.next_stuck_trigger_at:
                self.pending_full_cycle = f"任务 {task} 疑似卡死（{idle_minutes:.0f} 分钟无日志）"
                self.next_stuck_trigger_at = now + timedelta(seconds=self.settings.task_stuck_cooldown)
                return

    # ------------------------------------------------------------------
    # 控制文件 / 主循环
    # ------------------------------------------------------------------
    def handle_flags(self) -> None:
        if FLAG_STOP.exists():
            try:
                FLAG_STOP.unlink()
            except OSError:
                pass
            self.logger.info("收到停止请求，准备退出（若正在重启会等本次重启结束）")
            self.stop_requested = True
            return
        if FLAG_FULL_RESTART.exists():
            try:
                FLAG_FULL_RESTART.unlink()
            except OSError:
                pass
            self.pending_full_cycle = "手动请求完整重启"
        if FLAG_ALAS_RESTART.exists():
            try:
                FLAG_ALAS_RESTART.unlink()
            except OSError:
                pass
            self.pending_alas_restart = "手动请求重启 Alas"

    def tick(self) -> None:
        now = datetime.now()
        self.handle_flags()

        if self.pending_full_cycle:
            reason = self.pending_full_cycle
            self.pending_full_cycle = None
            if self.run_full_cycle(reason):
                self.retry_at = None
                self.next_restart = self.compute_next_restart(datetime.now())
                self.logger.info(f"[计划] 下一次定时重启: {self.next_restart:%Y-%m-%d %H:%M:%S}")
            else:
                self.failures += 1
                delay = min(self.settings.retry_delay * (2 ** min(self.failures - 1, 4)), 1800)
                self.retry_at = datetime.now() + timedelta(seconds=delay)
                self.logger.warning(f"[计划] 重启失败，将在 {delay} 秒后重试（连续失败 {self.failures} 次）")
            return

        if self.pending_alas_restart:
            reason = self.pending_alas_restart
            self.pending_alas_restart = None
            if not self._emulator_quick_ok():
                self.pending_full_cycle = "重启 Alas 前发现模拟器/ADB 异常"
                return
            if self.restart_alas_only(reason):
                self.next_alas_restart_at = datetime.now() + timedelta(
                    seconds=self.settings.alas_restart_cooldown
                )
            else:
                self.retry_at = datetime.now() + timedelta(seconds=self.settings.retry_delay)
            return

        if self.retry_at is not None and now >= self.retry_at:
            self.retry_at = None
            self.pending_full_cycle = "失败重试"
            return

        if self.next_restart is not None and now >= self.next_restart:
            self.next_restart = None
            self.pending_full_cycle = "定时重启"
            return

        if self.next_health is None or now >= self.next_health:
            self.next_health = now + timedelta(seconds=self.settings.check_interval)
            self.health_check()

        if self.next_status_log is None or now >= self.next_status_log:
            self.next_status_log = now + timedelta(seconds=self.settings.status_log_seconds)
            self.log_status_snapshot()

    def run(self) -> None:
        self.print_layout()
        if self.settings.restart_on_start:
            self.logger.info("[计划] restart_on_start = true，启动后立即完整重启")
            self.pending_full_cycle = "启动时立即重启"
        else:
            now = datetime.now()
            self.next_restart, graced = apply_startup_grace(
                self.compute_next_restart(now), now, self.settings.startup_grace_minutes
            )
            if graced:
                self.logger.info(
                    f"[计划] 模拟器已运行较久，计划重启时间已到；"
                    f"为安全起见等待 {self.settings.startup_grace_minutes} 分钟后再执行"
                )
            self.logger.info(f"[计划] 下一次定时重启: {self.next_restart:%Y-%m-%d %H:%M:%S}")
            if self.settings.ensure_running_on_start:
                try:
                    self.ensure_running_on_start()
                except Exception as exc:
                    self.logger.error(f"[启动] 初始环境准备失败: {exc}")
                    self.logger.debug(traceback.format_exc())

        self.logger.info("[运行] 看门狗已开始运行")
        self.next_status_log = datetime.now() + timedelta(seconds=self.settings.status_log_seconds)
        self.next_health = datetime.now() + timedelta(seconds=self.settings.check_interval)
        self.log_status_snapshot()
        while not self.stop_requested:
            try:
                self.tick()
            except KeyboardInterrupt:
                self.logger.info("收到 Ctrl+C，准备退出")
                break
            except Exception as exc:
                self.logger.error(f"[运行] 主循环异常: {exc}")
                self.logger.debug(traceback.format_exc())
            if self.stop_requested:
                break
            time.sleep(1.5)
        self.logger.info("[运行] 看门狗已退出")

    # ------------------------------------------------------------------
    # 状态输出
    # ------------------------------------------------------------------
    def collect_status(self) -> dict:
        return build_status(
            self.layout,
            self.mumu,
            self.alas,
            self.logwatch,
            next_restart=self.next_restart,
            cycle_count=self.cycle_count,
            failures=self.failures,
            last_cycle=self.last_cycle,
            started_at=self.started_at,
        )

    def write_status_json(self, payload: Optional[dict] = None) -> None:
        write_status(self.settings, payload or self.collect_status())

    def log_status_snapshot(self) -> None:
        payload = self.collect_status()
        for line in format_status_lines(payload, self.next_restart):
            if line.startswith("[状态] 模拟器查询失败") or line.startswith("[状态] ADB 查询失败"):
                self.logger.warning(line)
            else:
                self.logger.info(line)
        self.write_status_json(payload)

    def _log_emulator_memory(self, tag: str) -> None:
        try:
            from .win32api import proc_memory

            info = self.mumu.info()
            pid = info.get("pid")
            if not pid:
                return
            working_set, private = proc_memory(int(pid))
            if working_set:
                self.logger.info(
                    f"[{tag}] MuMu 内存: 工作集 {working_set / 2 ** 30:.2f} GB, "
                    f"提交 {private / 2 ** 30:.2f} GB"
                )
        except Exception:
            pass

    def print_layout(self) -> None:
        self.logger.info("-" * 70)
        self.logger.info(f"{APP_NAME} v{VERSION}  ({'exe' if FROZEN else 'python'})")
        self.logger.info(f"[配置] {self.settings.path}")
        self.logger.info(f"[路径] Alas: {self.layout.alas_root}")
        self.logger.info(f"[路径] MuMuManager: {self.layout.mumu_manager}")
        self.logger.info(
            f"[路径] 实例 {self.layout.instance}"
            + (f"（{self.layout.instance_name}）" if self.layout.instance_name else "")
            + f"  ADB: {self.mumu.serial}"
        )
        self.logger.info(f"[路径] 配置: {', '.join(self.layout.configs)} | WebUI 端口: {self.layout.webui_port}")
        self.logger.info(f"[路径] 启动方式: {self.layout.launch_mode} -> {self.layout.launch_exe}")
        self.logger.info(f"[路径] adb.exe: {self.layout.adb_exe}")
        self.logger.info(f"[日志] 目录: {self.settings.log_dir}（保留 {self.settings.keep_days} 天）")
        if self.settings.restart_times:
            times = ", ".join(f"{hour:02d}:{minute:02d}" for hour, minute in self.settings.restart_times)
            self.logger.info(f"[计划] 每天 {times} 执行完整重启")
        else:
            self.logger.info(f"[计划] 每 {self.settings.restart_interval_hours:g} 小时执行完整重启")
        self.logger.info("-" * 70)

    def print_status(self) -> None:
        from .util import last_line_with, tail_lines

        print("=" * 78)
        print(f"MuMuAlasWatchdog  状态检查    时间: {now_text()}")
        print("=" * 78)
        from .lockfile import watcher_running_pid

        pid = watcher_running_pid()
        print(f"看门狗进程: {'运行中，PID ' + str(pid) if pid else '未运行'}")
        print(f"配置文件:   {self.settings.path}")
        print(f"日志目录:   {self.settings.log_dir}")
        print(f"状态文件:   {STATUS_JSON}")
        print("-" * 78)
        print(f"Alas 目录:      {self.layout.alas_root}")
        print(f"MuMuManager:    {self.layout.mumu_manager}")
        print(f"MuMu 实例:      {self.layout.instance}" + (f"（{self.layout.instance_name}）" if self.layout.instance_name else ""))
        print(f"Alas 配置:      {', '.join(self.layout.configs)}")
        print(f"WebUI 端口:     {self.layout.webui_port}")
        print(f"ADB 可执行文件: {self.layout.adb_exe}")
        print(f"ADB serial:     {self.mumu.serial}")
        print(f"Alas 启动方式:  {self.layout.launch_mode} -> {self.layout.launch_exe}")
        print("-" * 78)

        try:
            info = self.mumu.info(timeout=20)
            print("[MuMu 模拟器]")
            for key, label in (
                ("name", "名称"),
                ("index", "实例编号"),
                ("is_process_started", "进程已启动"),
                ("is_android_started", "Android 已启动"),
                ("player_state", "状态"),
                ("pid", "进程 PID"),
                ("adb_host_ip", "ADB Host"),
                ("adb_port", "ADB 端口"),
            ):
                if key in info:
                    print(f"  {label:<14}: {info.get(key)}")
            pid_value = info.get("pid")
            if pid_value:
                from .win32api import proc_create_time, proc_memory

                working_set, private = proc_memory(int(pid_value))
                if working_set:
                    print(f"  {'内存':<14}: 工作集 {working_set / 2 ** 30:.2f} GB / 提交 {private / 2 ** 30:.2f} GB")
                    created = proc_create_time(int(pid_value))
                    if created:
                        print(
                            f"  {'启动时间':<14}: {created:%Y-%m-%d %H:%M:%S}"
                            f"（已运行 {fmt_duration((datetime.now() - created).total_seconds())}）"
                        )
        except Exception as exc:
            print(f"[MuMu 模拟器] 查询失败: {exc}")

        print("-" * 78)
        print("[ADB]")
        result = self.mumu.adb(["devices"], timeout=20)
        for line in (result.out or "").strip().splitlines():
            print(f"  {line}")
        if not (result.out or "").strip():
            print(f"  adb devices 无输出: {result.err.strip()}")
        try:
            state, response_ms = self.mumu.timed_adb_state()
            print(f"  serial {self.mumu.serial} 状态: {state or '无响应'}（响应 {int(response_ms)} ms）")
            boot = self.mumu.adb_shell(["getprop", "sys.boot_completed"], timeout=15)
            print(f"  sys.boot_completed: {(boot.out or '').strip() or '无响应'}")
        except Exception as exc:
            print(f"  查询失败: {exc}")

        print("-" * 78)
        print("[Alas]")
        try:
            electron = self.alas.electron_processes()
            main = self.alas.electron_main()
            webui_ok, webui_ms = self.alas.webui_status()
            print(f"  Electron 进程数: {len(electron)}" + (f"，主进程 PID {main.pid}" if main else ""))
            print(f"  WebUI 访问: {'正常' if webui_ok else '异常'}（响应 {int(webui_ms)} ms）")
            task_info = self.logwatch.status_dict()
            if task_info.get("current_task"):
                print(
                    f"  当前任务: {task_info.get('current_task')}"
                    f"（开始于 {task_info.get('task_started')}，最近日志 {task_info.get('last_activity')}）"
                )
            else:
                print("  当前任务: 空闲/等待中")
            start_line = self.logwatch.last_start_line()
            if start_line:
                print(f"  最近调度器启动: {start_line[0]}: {start_line[1]}")
            else:
                print("  最近调度器启动: 未在日志中找到")
        except Exception as exc:
            print(f"  查询失败: {exc}")

        print("-" * 78)
        print("[计划]")
        try:
            self.refresh_emulator_runtime()
            next_restart = self.compute_next_restart(datetime.now())
            if self.settings.restart_times:
                times = ", ".join(f"{hour:02d}:{minute:02d}" for hour, minute in self.settings.restart_times)
                print(f"  重启模式: 每天 {times}")
            else:
                print(f"  重启模式: 每 {self.settings.restart_interval_hours:g} 小时")
            if next_restart >= datetime.now():
                print(f"  预计下次重启: {next_restart:%Y-%m-%d %H:%M:%S}")
            else:
                print(
                    f"  计划重启时间: {next_restart:%Y-%m-%d %H:%M:%S}（已到期，"
                    f"看门狗启动后等待 {self.settings.startup_grace_minutes} 分钟宽限执行）"
                )
        except Exception as exc:
            print(f"  计算失败: {exc}")
        print("=" * 78)


__all__ = ["Watchdog"]
