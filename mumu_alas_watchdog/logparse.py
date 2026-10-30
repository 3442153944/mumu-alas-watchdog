"""Alas 日志解析（纯函数，方便单元测试）。"""

from __future__ import annotations

import re
from dataclasses import dataclass
from datetime import datetime
from typing import Optional

LOG_TIME_RE = re.compile(r"^(\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3})")
TASK_START_RE = re.compile(r"Scheduler: Start task `([^`]+)`")
TASK_END_RE = re.compile(r"Scheduler: End task `[^`]*`")


@dataclass
class TaskState:
    """Alas 当前任务状态（用于状态展示与卡死判断）。"""

    current_task: Optional[str] = None
    task_started: Optional[datetime] = None
    last_activity: Optional[datetime] = None
    last_log_file: Optional[str] = None
    last_line: str = ""


def parse_log_time(line: str) -> Optional[datetime]:
    match = LOG_TIME_RE.match(line)
    if not match:
        return None
    try:
        return datetime.strptime(match.group(1), "%Y-%m-%d %H:%M:%S.%f")
    except ValueError:
        return None


def update_task_state(state: TaskState, text: str, now: datetime) -> None:
    """用新增日志更新任务状态：Start task / End task / 最后活动时间。"""
    for line in text.splitlines():
        if not line.strip():
            continue
        started = TASK_START_RE.search(line)
        if started:
            state.current_task = started.group(1)
            state.task_started = parse_log_time(line) or now
        if TASK_END_RE.search(line):
            state.current_task = None
            state.task_started = None
        state.last_activity = now
        state.last_line = line.strip()[:200]


def scheduler_started(text: str, config_name: str) -> bool:
    return f"Start scheduler loop: {config_name}" in text


def detect_manual_stop(text: str, config_name: str) -> bool:
    return f"[{config_name}] exited" in text


__all__ = [
    "TaskState",
    "parse_log_time",
    "update_task_state",
    "scheduler_started",
    "detect_manual_stop",
]
