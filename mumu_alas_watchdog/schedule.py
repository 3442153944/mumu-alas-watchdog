"""重启计划计算（纯函数，方便单元测试）。"""

from __future__ import annotations

from datetime import datetime, timedelta
from typing import List, Optional, Tuple


def compute_next_restart(
    now: datetime,
    restart_times: List[Tuple[int, int]],
    interval_hours: float,
    emulator_start_time: Optional[datetime],
) -> datetime:
    """返回下一次计划重启时间。

    - restart_times 非空：按每天固定时间点（今天还没到就今天，否则明天）；
    - 否则按 interval_hours 间隔，基准是模拟器进程启动时间（未知则用 now）。
    """
    if restart_times:
        for hour, minute in restart_times:
            candidate = now.replace(hour=hour, minute=minute, second=0, microsecond=0)
            if candidate > now + timedelta(seconds=30):
                return candidate
        hour, minute = restart_times[0]
        tomorrow = now + timedelta(days=1)
        return tomorrow.replace(hour=hour, minute=minute, second=0, microsecond=0)

    interval = timedelta(hours=max(0.1, interval_hours))
    base = emulator_start_time or now
    return base + interval


def apply_startup_grace(
    next_restart: datetime, now: datetime, grace_minutes: int
) -> tuple[datetime, bool]:
    """看门狗启动时，若计划时间过近则顺延到宽限期之后。返回 (时间, 是否顺延)。"""
    grace = now + timedelta(minutes=max(0, grace_minutes))
    if next_restart < grace:
        return grace, True
    return next_restart, False


__all__ = ["compute_next_restart", "apply_startup_grace"]
