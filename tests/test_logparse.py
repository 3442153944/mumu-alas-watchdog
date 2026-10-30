from __future__ import annotations

from datetime import datetime

from mumu_alas_watchdog.logparse import (
    TaskState,
    detect_manual_stop,
    parse_log_time,
    scheduler_started,
    update_task_state,
)


def test_parse_log_time():
    line = "2026-10-30 15:30:54.464 | INFO | Bind task ['General', 'Alas', 'Main']"
    assert parse_log_time(line) == datetime(2026, 10, 30, 15, 30, 54, 464000)
    assert parse_log_time("no timestamp") is None


def test_task_state_start_end():
    state = TaskState()
    now = datetime(2026, 10, 30, 16, 0, 0)
    update_task_state(
        state,
        "2026-10-30 15:15:08.439 | INFO | Scheduler: Start task `Main`\n",
        now,
    )
    assert state.current_task == "Main"
    assert state.task_started == datetime(2026, 10, 30, 15, 15, 8, 439000)
    assert state.last_activity == now

    update_task_state(
        state,
        "2026-10-30 15:20:00.000 | INFO | Scheduler: End task `Main`\n",
        now,
    )
    assert state.current_task is None
    assert state.task_started is None


def test_task_state_keeps_last_line_and_activity():
    state = TaskState()
    now = datetime(2026, 10, 30, 16, 0, 0)
    update_task_state(state, "\n", now)
    assert state.last_activity is None
    update_task_state(state, "2026-10-30 15:00:00.000 | INFO | something\n", now)
    assert state.last_activity == now
    assert "something" in state.last_line


def test_markers():
    assert scheduler_started("2026-10-30 14:05:42.256 | INFO | Start scheduler loop: alas", "alas")
    assert not scheduler_started("Start scheduler loop: other", "alas")
    assert detect_manual_stop("2026-10-30 15:31:42 | INFO | [alas] exited", "alas")
    assert not detect_manual_stop("[other] exited", "alas")
