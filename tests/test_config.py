from __future__ import annotations

from pathlib import Path

import pytest

from mumu_alas_watchdog.config import DEFAULT_CONFIG_TEXT, Settings, ensure_config_file


def test_ensure_config_file_creates_default(tmp_path: Path):
    path = tmp_path / "config.yml"
    assert ensure_config_file(path)
    assert path.exists()
    assert path.read_text(encoding="utf-8") == DEFAULT_CONFIG_TEXT
    assert not ensure_config_file(path)


def test_defaults_are_loaded_when_fields_missing(tmp_path: Path):
    path = tmp_path / "config.yml"
    path.write_text("paths:\n  alas_root: D:/Alas\n", encoding="utf-8")
    settings = Settings(path)
    assert settings.alas_root == "D:/Alas"
    assert settings.mumu_manager == "auto"
    assert settings.restart_interval_hours == 6
    assert settings.launch_mode == "electron"
    assert settings.restart_times == []
    assert settings.log_dir == Path(__file__).resolve().parent.parent / "logs"


def test_user_values_override_defaults(tmp_path: Path):
    path = tmp_path / "config.yml"
    path.write_text(
        """
paths:
  alas_root: auto
  launch_mode: launcher
alas:
  config_names: [alas, alas2]
schedule:
  restart_times: ["04:00", "16:30"]
  restart_interval_hours: 8
  ensure_running_on_start: false
health:
  check_interval_seconds: 90
  task_stuck_minutes: 0
timeouts:
  adb_ready_seconds: 120
logging:
  dir: mylogs
  keep_days: 3
""",
        encoding="utf-8",
    )
    settings = Settings(path)
    assert settings.launch_mode == "launcher"
    assert settings.config_names == ["alas", "alas2"]
    assert settings.restart_times == [(4, 0), (16, 30)]
    assert settings.restart_interval_hours == 8
    assert settings.ensure_running_on_start is False
    assert settings.check_interval == 90
    assert settings.task_stuck_minutes == 0
    assert settings.adb_ready_timeout == 120
    assert settings.log_dir == Path(__file__).resolve().parent.parent / "mylogs"
    assert settings.keep_days == 3


def test_invalid_yaml_raises(tmp_path: Path):
    path = tmp_path / "config.yml"
    path.write_text("paths: [unclosed", encoding="utf-8")
    with pytest.raises(ValueError):
        Settings(path)


def test_top_level_must_be_mapping(tmp_path: Path):
    path = tmp_path / "config.yml"
    path.write_text("- a\n- b\n", encoding="utf-8")
    with pytest.raises(ValueError):
        Settings(path)
