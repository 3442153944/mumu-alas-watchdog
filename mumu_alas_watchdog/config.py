"""配置读取（config.yml）、Alas 自身配置探测辅助。"""

from __future__ import annotations

import re
from pathlib import Path
from typing import Dict

import yaml

from .constants import WD_DIR
from .util import parse_hhmm_list

DEFAULT_CONFIG_TEXT = """\
# ============================================================================
# MuMu + Alas 自动重启看门狗 配置文件 (YAML)
# 修改后需要重启看门狗（--stop 然后 --start）才生效。
# 路径建议使用正斜杠 / ；auto 表示自动识别。
# ============================================================================

paths:
  # Alas 根目录（包含 alas.py / gui.py）。auto = 自动识别（含从运行中的 Alas 进程推断）
  alas_root: auto
  # MuMuManager.exe。auto = 从 Alas 配置 / 运行中的 MuMu 进程自动识别
  mumu_manager: auto
  # MuMu 实例编号。auto = 从 Alas 配置的模拟器名 / ADB 端口自动识别
  mumu_instance: auto
  # adb.exe。auto = 读取 Alas deploy.yaml 的 AdbExecutable
  adb_exe: auto
  # electron = 直接启动 toolkit/webapp/alas.exe（推荐）；launcher = 启动 Alas.exe
  launch_mode: electron

alas:
  # 需要自动恢复的 Alas 配置名（对应 config/<名字>.json）
  config_names: auto

schedule:
  # 每天固定时间重启，例如 ["04:00", "16:00"]；为空则按 restart_interval_hours 间隔
  restart_times: []
  restart_interval_hours: 6
  # 看门狗启动后是否立刻完整重启一次
  restart_on_start: false
  # 启动时若模拟器/Alas 未运行，是否自动拉起
  ensure_running_on_start: true
  # 重启失败后的重试间隔（秒），连续失败自动加倍，最长 1800 秒
  retry_delay_seconds: 300
  # 看门狗启动后第一次计划重启的最小等待（分钟），避免刚启动就重启
  startup_grace_minutes: 10

health:
  # 巡检间隔（秒）
  check_interval_seconds: 60
  # ADB 连续失败多少次后触发完整重启
  adb_fail_threshold: 3
  # 模拟器进程提交内存超过该值提前重启；0 = 关闭
  mumu_memory_limit_gb: 0
  # 模拟器意外退出时是否自动重启
  restart_if_emulator_down: true
  # Alas 意外退出/崩溃时是否自动重新拉起
  restart_alas_if_closed: true
  # Alas 自动重新拉起的最小间隔（秒）
  alas_restart_cooldown_seconds: 600
  # 当前任务超过多少分钟没有任何新日志视为疑似卡死；0 = 关闭
  task_stuck_minutes: 60
  # 疑似卡死时是否自动完整重启
  restart_on_task_stuck: true
  # 卡死重启的最小间隔（秒）
  task_stuck_cooldown_seconds: 3600

timeouts:
  emulator_shutdown_seconds: 90
  emulator_boot_seconds: 240
  adb_ready_seconds: 300
  alas_start_seconds: 180
  scheduler_start_seconds: 180

process:
  # 重启前是否结束 adb server（清除卡死的 ADB 连接）
  kill_adb_server: true
  # 重启 Alas 前写 config/reloadalas，让它自动开始游戏脚本
  use_reloadalas: true
  # 结束 Alas 进程最长等待（秒）
  kill_alas_seconds: 30

monitor:
  # 运行状态日志输出间隔（秒）
  status_log_seconds: 300
  # 是否写 status.json
  write_status_json: true

logging:
  # 日志目录（相对路径基于程序所在目录）
  dir: logs
  # 日志保留天数
  keep_days: 14
  # 日志级别: DEBUG / INFO / WARNING / ERROR
  level: INFO

advanced:
  # 单条 MuMuManager 命令超时（秒）
  mumu_cmd_timeout_seconds: 90
"""


def ensure_config_file(path: Path) -> bool:
    """首次运行生成 config.yml：优先复制 config.example.yml，否则写入内置默认文本。"""
    if path.exists():
        return False
    path.parent.mkdir(parents=True, exist_ok=True)
    example = WD_DIR / "config.example.yml"
    try:
        text = example.read_text(encoding="utf-8")
    except OSError:
        text = DEFAULT_CONFIG_TEXT
    path.write_text(text, encoding="utf-8")
    return True


def _yaml_get(data, *keys, default=None):
    current = data
    for key in keys:
        if not isinstance(current, dict) or key not in current:
            return default
        current = current[key]
    if current is None:
        return default
    return current


class Settings:
    """config.yml 的强类型视图；所有字段都有默认值。"""

    def __init__(self, path: Path):
        self.path = path
        self.created = ensure_config_file(path)
        try:
            raw = yaml.safe_load(path.read_text(encoding="utf-8", errors="replace")) or {}
        except yaml.YAMLError as exc:
            raise ValueError(f"config.yml 解析失败: {exc}")
        if not isinstance(raw, dict):
            raise ValueError("config.yml 顶层必须是映射（key: value）")
        self.raw = raw

        def text(*keys, default: str = "") -> str:
            value = _yaml_get(raw, *keys, default=default)
            return default if value is None else str(value).strip()

        def number(*keys, default: float) -> float:
            value = _yaml_get(raw, *keys, default=default)
            try:
                return float(value)
            except (TypeError, ValueError):
                return default

        def integer(*keys, default: int) -> int:
            return int(number(*keys, default=float(default)))

        def boolean(*keys, default: bool) -> bool:
            value = _yaml_get(raw, *keys, default=default)
            if isinstance(value, bool):
                return value
            lowered = str(value).strip().lower()
            if lowered in ("1", "true", "yes", "on"):
                return True
            if lowered in ("0", "false", "no", "off"):
                return False
            return default

        # paths
        self.alas_root = text("paths", "alas_root", default="auto")
        self.mumu_manager = text("paths", "mumu_manager", default="auto")
        self.mumu_instance = text("paths", "mumu_instance", default="auto")
        self.adb_exe = text("paths", "adb_exe", default="auto")
        self.launch_mode = text("paths", "launch_mode", default="electron").lower()
        if self.launch_mode not in ("electron", "launcher"):
            self.launch_mode = "electron"

        # alas
        configured_names = _yaml_get(raw, "alas", "config_names", default="auto")
        if isinstance(configured_names, (list, tuple)):
            self.config_names = [str(item).strip() for item in configured_names if str(item).strip()]
        else:
            self.config_names = text("alas", "config_names", default="auto")

        # schedule
        self.restart_times = parse_hhmm_list(_yaml_get(raw, "schedule", "restart_times", default=[]))
        self.restart_interval_hours = max(0.1, number("schedule", "restart_interval_hours", default=6))
        self.restart_on_start = boolean("schedule", "restart_on_start", default=False)
        self.ensure_running_on_start = boolean("schedule", "ensure_running_on_start", default=True)
        self.retry_delay = max(30, integer("schedule", "retry_delay_seconds", default=300))
        self.startup_grace_minutes = max(0, integer("schedule", "startup_grace_minutes", default=10))

        # health
        self.check_interval = max(20, integer("health", "check_interval_seconds", default=60))
        self.adb_fail_threshold = max(2, integer("health", "adb_fail_threshold", default=3))
        self.mumu_memory_limit_gb = max(0.0, number("health", "mumu_memory_limit_gb", default=0))
        self.restart_if_emulator_down = boolean("health", "restart_if_emulator_down", default=True)
        self.restart_alas_if_closed = boolean("health", "restart_alas_if_closed", default=True)
        self.alas_restart_cooldown = max(60, integer("health", "alas_restart_cooldown_seconds", default=600))
        self.task_stuck_minutes = max(0, integer("health", "task_stuck_minutes", default=60))
        self.restart_on_task_stuck = boolean("health", "restart_on_task_stuck", default=True)
        self.task_stuck_cooldown = max(300, integer("health", "task_stuck_cooldown_seconds", default=3600))

        # timeouts
        self.emulator_shutdown_timeout = integer("timeouts", "emulator_shutdown_seconds", default=90)
        self.emulator_boot_timeout = integer("timeouts", "emulator_boot_seconds", default=240)
        self.adb_ready_timeout = integer("timeouts", "adb_ready_seconds", default=300)
        self.alas_start_timeout = integer("timeouts", "alas_start_seconds", default=180)
        self.scheduler_start_timeout = integer("timeouts", "scheduler_start_seconds", default=180)

        # process
        self.kill_adb_server = boolean("process", "kill_adb_server", default=True)
        self.use_reloadalas = boolean("process", "use_reloadalas", default=True)
        self.kill_alas_timeout = max(5, integer("process", "kill_alas_seconds", default=30))

        # monitor
        self.status_log_seconds = max(30, integer("monitor", "status_log_seconds", default=300))
        self.write_status_json = boolean("monitor", "write_status_json", default=True)

        # logging
        log_dir = text("logging", "dir", default="logs") or "logs"
        log_dir_path = Path(log_dir)
        if not log_dir_path.is_absolute():
            log_dir_path = WD_DIR / log_dir_path
        self.log_dir = log_dir_path
        self.keep_days = max(1, integer("logging", "keep_days", default=14))
        self.log_level = text("logging", "level", default="INFO").upper() or "INFO"

        # advanced
        self.mumu_cmd_timeout = max(20, integer("advanced", "mumu_cmd_timeout_seconds", default=90))


def read_deploy_config(root: Path) -> Dict[str, str]:
    """用轻量正则读取 deploy.yaml 的少量标量配置（不依赖 PyYAML 的结构）。"""
    result: Dict[str, str] = {}
    path = root / "config" / "deploy.yaml"
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except OSError:
        return result
    for line in text.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#") or ":" not in stripped:
            continue
        key, value = stripped.split(":", 1)
        key = key.strip()
        value = value.split("#", 1)[0].strip().strip('"').strip("'")
        if re.fullmatch(r"[A-Za-z0-9_]+", key) and key not in result:
            result[key] = value
    return result


def read_alas_config(root: Path) -> dict:
    import json

    path = root / "config" / "alas.json"
    try:
        data = json.loads(path.read_text(encoding="utf-8", errors="replace"))
        return data if isinstance(data, dict) else {}
    except Exception:
        return {}


def resolve_relative(root: Path, value: str) -> Path:
    path = Path(value)
    return path if path.is_absolute() else root / path


__all__ = [
    "DEFAULT_CONFIG_TEXT",
    "Settings",
    "ensure_config_file",
    "read_deploy_config",
    "read_alas_config",
    "resolve_relative",
]
