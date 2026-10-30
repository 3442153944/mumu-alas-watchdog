"""Alas 日志增量读取、任务状态跟踪、调度器启动等待。"""

from __future__ import annotations

import time
from datetime import datetime
from pathlib import Path
from typing import Callable, Dict, List, Optional, Tuple

from .config import Settings
from .constants import FATAL_LOG_PATTERNS
from .layout import Layout
from .logparse import TaskState, detect_manual_stop, parse_log_time, scheduler_started, update_task_state
from .util import last_line_with, tail_lines


class LogWatcher:
    def __init__(self, settings: Settings, layout: Layout, logger):
        self.settings = settings
        self.layout = layout
        self.logger = logger
        self.log_dir = layout.alas_log_dir
        self.configs = layout.configs
        self.offsets: Dict[str, int] = {}
        self.task_state: Dict[str, TaskState] = {name: TaskState() for name in self.configs}
        self.manual_stopped = False
        self.manual_warned = False
        self._seen_keys = set()
        self.bootstrap()
        self.prime_offsets()

    # ------------------------------------------------------------------
    # 增量读取
    # ------------------------------------------------------------------
    def _log_files(self) -> List[Path]:
        try:
            return sorted(self.log_dir.glob("*.txt"))
        except OSError:
            return []

    def prime_offsets(self) -> None:
        self.offsets = {}
        for path in self._log_files():
            try:
                self.offsets[str(path)] = path.stat().st_size
            except OSError:
                pass

    def read_appended(self, offsets: Dict[str, int]) -> Dict[str, str]:
        result: Dict[str, str] = {}
        for path in self._log_files():
            try:
                size = path.stat().st_size
            except OSError:
                continue
            key = str(path)
            base = offsets.get(key, 0)
            if size < base:
                base = 0
            if size == base:
                offsets[key] = size
                continue
            try:
                with open(path, "rb") as handle:
                    handle.seek(base)
                    data = handle.read(size - base)
            except OSError:
                continue
            offsets[key] = size
            result[path.name] = data.decode("utf-8", "replace")
        return result

    def scan(self) -> Dict[str, str]:
        return self.read_appended(self.offsets)

    # ------------------------------------------------------------------
    # 启动时恢复任务状态
    # ------------------------------------------------------------------
    def bootstrap(self) -> None:
        for config_name, state in self.task_state.items():
            candidates = sorted(
                self.log_dir.glob(f"*_{config_name}.txt"),
                key=lambda item: item.stat().st_mtime if item.exists() else 0,
                reverse=True,
            )
            if not candidates:
                continue
            path = candidates[0]
            try:
                mtime = datetime.fromtimestamp(path.stat().st_mtime)
            except OSError:
                mtime = datetime.now()
            state.last_activity = mtime
            state.last_log_file = path.name
            for line in tail_lines(path, 800):
                started = __import__("re").search(r"Scheduler: Start task `([^`]+)`", line)
                if started:
                    state.current_task = started.group(1)
                    state.task_started = parse_log_time(line) or mtime
                if __import__("re").search(r"Scheduler: End task `", line):
                    state.current_task = None
                    state.task_started = None
                if line.strip():
                    state.last_line = line.strip()[:200]

    # ------------------------------------------------------------------
    # 分析新增日志
    # ------------------------------------------------------------------
    def analyze(self, texts: Dict[str, str]) -> None:
        now = datetime.now()
        for file_name, text in texts.items():
            if not text:
                continue
            for config_name, state in self.task_state.items():
                if file_name.endswith(f"_{config_name}.txt"):
                    update_task_state(state, text, now)
                    state.last_log_file = file_name

            for pattern in FATAL_LOG_PATTERNS:
                if pattern not in text:
                    continue
                for line in text.splitlines():
                    if pattern not in line:
                        continue
                    key = (pattern, line.strip()[:160])
                    if key in self._seen_keys:
                        continue
                    if len(self._seen_keys) > 2000:
                        self._seen_keys.clear()
                    self._seen_keys.add(key)
                    self.logger.warning(f"[Alas] 日志告警 [{file_name}]: {line.strip()[:180]}")

            for config_name in self.configs:
                if scheduler_started(text, config_name):
                    self.manual_stopped = False
                    self.manual_warned = False
                if detect_manual_stop(text, config_name) and not self.manual_warned:
                    self.manual_stopped = True
                    self.manual_warned = True
                    self.logger.info(
                        "[Alas] 检测到调度器被手动停止，自动恢复已暂停"
                        "（下次计划重启或 --restart-alas-now 会恢复）"
                    )
            if "Alas closed." in text and not self.manual_warned:
                self.manual_stopped = True
                self.manual_warned = True
                self.logger.info("[Alas] 检测到 Alas 被手动关闭，自动恢复已暂停（下次计划重启会自动恢复）")

    # ------------------------------------------------------------------
    # 等待调度器自动开始
    # ------------------------------------------------------------------
    def wait_scheduler_started(
        self,
        offsets: Dict[str, int],
        electron_alive: Callable[[], bool],
        webui_ok: Callable[[], bool],
    ) -> None:
        self.logger.info("[Alas] 等待游戏脚本（调度器）自动启动...")
        pending = list(self.configs)
        deadline = time.time() + self.settings.scheduler_start_timeout
        last_report = 0.0
        while pending and time.time() < deadline:
            texts = self.read_appended(offsets)
            for text in texts.values():
                for config_name in list(pending):
                    if scheduler_started(text, config_name):
                        pending.remove(config_name)
                        self.logger.info(f"[Alas] 游戏脚本已自动开始: {config_name}")
            if not pending:
                break
            if not electron_alive() and not webui_ok():
                raise RuntimeError("Alas 在启动过程中意外退出")
            if time.time() - last_report > 15:
                last_report = time.time()
                self.logger.info(f"[Alas] 仍在等待调度器启动，未完成: {pending}")
            time.sleep(2)
        if pending:
            raise TimeoutError(
                f"等待调度器自动启动超时（{self.settings.scheduler_start_timeout} 秒），"
                f"未完成: {pending}。请检查 use_reloadalas 配置以及 Alas 日志"
            )
        self.manual_stopped = False
        self.manual_warned = False

    # ------------------------------------------------------------------
    # 辅助
    # ------------------------------------------------------------------
    def last_start_line(self) -> Optional[Tuple[str, str]]:
        for config_name in self.configs:
            candidates = sorted(
                self.log_dir.glob(f"*_{config_name}.txt"),
                key=lambda item: item.stat().st_mtime if item.exists() else 0,
                reverse=True,
            )
            for path in candidates:
                line = last_line_with(path, "Start scheduler loop:")
                if line:
                    return path.name, line
        return None

    def current_task(self) -> Tuple[Optional[str], Optional[TaskState]]:
        for config_name, state in self.task_state.items():
            if state.current_task:
                return config_name, state
        return None, None

    def status_dict(self) -> dict:
        config_name, state = self.current_task()
        if not state:
            return {"current_task": None, "config": None}
        return {
            "config": config_name,
            "current_task": state.current_task,
            "task_started": state.task_started.strftime("%Y-%m-%d %H:%M:%S") if state.task_started else None,
            "last_activity": state.last_activity.strftime("%Y-%m-%d %H:%M:%S") if state.last_activity else None,
            "manual_stopped": self.manual_stopped,
        }


__all__ = ["LogWatcher"]
