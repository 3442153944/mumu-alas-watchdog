from __future__ import annotations

from datetime import datetime, timedelta

from mumu_alas_watchdog.schedule import apply_startup_grace, compute_next_restart


def test_fixed_times_pick_next_future_slot():
    now = datetime(2026, 1, 1, 10, 0, 0)
    result = compute_next_restart(now, [(4, 0), (16, 0)], 6, None)
    assert result == datetime(2026, 1, 1, 16, 0, 0)


def test_fixed_times_roll_to_tomorrow():
    now = datetime(2026, 1, 1, 20, 0, 0)
    result = compute_next_restart(now, [(4, 0), (16, 0)], 6, None)
    assert result == datetime(2026, 1, 2, 4, 0, 0)


def test_interval_based_on_emulator_start_time():
    now = datetime(2026, 1, 1, 12, 0, 0)
    started = now - timedelta(hours=2)
    result = compute_next_restart(now, [], 6, started)
    assert result == now + timedelta(hours=4)


def test_interval_without_known_start():
    now = datetime(2026, 1, 1, 12, 0, 0)
    result = compute_next_restart(now, [], 6, None)
    assert result == now + timedelta(hours=6)


def test_startup_grace_pushes_imminent_restart():
    now = datetime(2026, 1, 1, 12, 0, 0)
    result, graced = apply_startup_grace(now + timedelta(minutes=1), now, 10)
    assert graced
    assert result == now + timedelta(minutes=10)

    later = now + timedelta(hours=2)
    result, graced = apply_startup_grace(later, now, 10)
    assert not graced
    assert result == later
