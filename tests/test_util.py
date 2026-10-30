from __future__ import annotations

from pathlib import Path

from mumu_alas_watchdog.util import (
    fmt_duration,
    parse_hhmm_list,
    path_is_under,
    same_path,
)
from mumu_alas_watchdog.win32api import build_env_block, clean_launch_env


def test_parse_hhmm_list_filters_invalid_values():
    assert parse_hhmm_list("04:00, 16:30;bad;25:00") == [(4, 0), (16, 30)]
    assert parse_hhmm_list(["23:59", "00:00"]) == [(0, 0), (23, 59)]
    assert parse_hhmm_list(None) == []


def test_fmt_duration():
    assert fmt_duration(0) == "0秒"
    assert fmt_duration(59) == "59秒"
    assert fmt_duration(60) == "1分0秒"
    assert fmt_duration(3661) == "1小时1分1秒"


def test_path_helpers(tmp_path: Path):
    base = tmp_path / "Alas"
    child = base / "toolkit" / "webapp" / "alas.exe"
    assert path_is_under(child, base)
    assert same_path(base, tmp_path / "Alas")
    assert not path_is_under(tmp_path / "Other", base)


def test_env_block_is_double_null_terminated():
    buffer = build_env_block({"A": "1"})
    text = "".join(buffer)
    assert "A=1" in text
    assert text.endswith("\x00\x00")


def test_clean_launch_env_filters_framework_variables(monkeypatch):
    monkeypatch.setenv("ELECTRON_RUN_AS_NODE", "1")
    monkeypatch.setenv("ELECTRON_NO_ATTACH_CONSOLE", "1")
    monkeypatch.setenv("MUW_CMD_LOG", "x.log")
    monkeypatch.setenv("_PYI_APPLICATION_HOME_DIR", "C:/Temp/_MEI")
    monkeypatch.setenv("PYINSTALLER_TEST", "1")
    monkeypatch.setenv("KEEP_ME", "yes")

    env = clean_launch_env()
    assert "ELECTRON_RUN_AS_NODE" not in env
    assert "ELECTRON_NO_ATTACH_CONSOLE" not in env
    assert "MUW_CMD_LOG" not in env
    assert "_PYI_APPLICATION_HOME_DIR" not in env
    assert "PYINSTALLER_TEST" not in env
    assert env["KEEP_ME"] == "yes"
