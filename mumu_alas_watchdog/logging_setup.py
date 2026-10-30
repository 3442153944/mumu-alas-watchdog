"""日志初始化与清理。"""

from __future__ import annotations

import logging
import sys
import time
from datetime import datetime
from pathlib import Path

from .constants import APP_NAME


def setup_logging(log_dir: Path, level: str, to_file: bool, console: bool = True) -> logging.Logger:
    logger = logging.getLogger(APP_NAME)
    logger.setLevel(getattr(logging, level, logging.INFO))
    logger.handlers.clear()
    logger.propagate = False
    formatter = logging.Formatter(
        "%(asctime)s | %(levelname)-7s | %(message)s",
        "%Y-%m-%d %H:%M:%S",
    )

    if console and sys.stdout is not None:
        console_handler = logging.StreamHandler(sys.stdout)
        console_handler.setFormatter(formatter)
        logger.addHandler(console_handler)

    if to_file:
        try:
            log_dir.mkdir(parents=True, exist_ok=True)
            file_handler = logging.FileHandler(
                log_dir / f"watchdog_{datetime.now():%Y%m%d}.log",
                encoding="utf-8",
            )
            file_handler.setFormatter(formatter)
            logger.addHandler(file_handler)
        except OSError:
            pass

    if not logger.handlers:
        logger.addHandler(logging.NullHandler())
    return logger


def prune_logs(log_dir: Path, keep_days: int) -> None:
    cutoff = time.time() - max(1, keep_days) * 86400
    try:
        files = list(log_dir.glob("watchdog_*.log"))
    except OSError:
        return
    for path in files:
        try:
            if path.stat().st_mtime < cutoff:
                path.unlink()
        except OSError:
            pass


__all__ = ["setup_logging", "prune_logs"]
