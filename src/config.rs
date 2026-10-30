//! 配置读取（config.yml），首次运行从 config.example.yml 复制。

use std::fs;
use std::path::{Path, PathBuf};

use serde_yaml::Value;

use crate::constants;
use crate::util;

pub const DEFAULT_CONFIG_TEXT: &str = include_str!("../config.example.yml");

#[derive(Debug, Clone)]
pub struct Settings {
    pub path: PathBuf,
    pub created: bool,

    pub alas_root: String,
    pub mumu_manager: String,
    pub mumu_instance: Option<u32>,
    pub adb_exe: String,
    pub launch_mode: String,
    pub config_names: Option<Vec<String>>,

    pub restart_times: Vec<(u32, u32)>,
    pub restart_interval_hours: f64,
    pub restart_on_start: bool,
    pub ensure_running_on_start: bool,
    pub retry_delay: u64,
    pub startup_grace_minutes: i64,

    pub check_interval: u64,
    pub adb_fail_threshold: u32,
    pub mumu_memory_limit_gb: f64,
    pub restart_if_emulator_down: bool,
    pub restart_alas_if_closed: bool,
    pub alas_restart_cooldown: i64,
    pub task_stuck_minutes: i64,
    pub restart_on_task_stuck: bool,
    pub task_stuck_cooldown: i64,

    pub emulator_shutdown_timeout: u64,
    pub emulator_boot_timeout: u64,
    pub adb_ready_timeout: u64,
    pub alas_start_timeout: u64,
    pub scheduler_start_timeout: u64,

    pub kill_adb_server: bool,
    pub use_reloadalas: bool,
    pub kill_alas_timeout: u64,

    pub status_log_seconds: u64,
    pub write_status_json: bool,
    pub self_restart_memory_mb: u64,
    pub self_restart_handles: u32,
    pub self_restart_after_hours: u64,

    pub log_dir: PathBuf,
    pub keep_days: u32,
    pub log_level: String,

    pub mumu_cmd_timeout: u64,
}

fn mapping_get<'a>(root: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = root;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
}

fn as_string(value: Option<&Value>, default: &str) -> String {
    value
        .and_then(|value| match value {
            Value::String(text) => Some(text.trim().to_string()),
            Value::Number(number) => Some(number.to_string()),
            Value::Bool(flag) => Some(flag.to_string()),
            _ => None,
        })
        .filter(|text| !text.is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn as_bool(value: Option<&Value>, default: bool) -> bool {
    match value {
        Some(Value::Bool(flag)) => *flag,
        Some(Value::String(text)) => matches!(text.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"),
        Some(Value::Number(number)) => number.as_i64().map(|n| n != 0).unwrap_or(default),
        _ => default,
    }
}

fn as_f64(value: Option<&Value>, default: f64) -> f64 {
    match value {
        Some(Value::Number(number)) => number.as_f64().unwrap_or(default),
        Some(Value::String(text)) => text.trim().parse::<f64>().unwrap_or(default),
        _ => default,
    }
}

fn as_u64(value: Option<&Value>, default: u64) -> u64 {
    let number = as_f64(value, default as f64);
    if number.is_finite() && number >= 0.0 {
        number as u64
    } else {
        default
    }
}

fn as_i64(value: Option<&Value>, default: i64) -> i64 {
    as_f64(value, default as f64) as i64
}

impl Settings {
    pub fn load(path: &Path) -> Result<Settings, String> {
        let mut created = false;
        if !path.exists() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|err| format!("创建配置目录失败: {err}"))?;
            }
            let example = constants::config_example_path();
            let text = fs::read_to_string(&example).unwrap_or_else(|_| DEFAULT_CONFIG_TEXT.to_string());
            fs::write(path, text).map_err(|err| format!("写入配置失败: {err}"))?;
            created = true;
        }
        let text = fs::read_to_string(path).map_err(|err| format!("读取配置失败: {err}"))?;
        let raw: Value = serde_yaml::from_str(&text).map_err(|err| format!("config.yml 解析失败: {err}"))?;
        if !raw.is_mapping() {
            return Err("config.yml 顶层必须是映射（key: value）".to_string());
        }

        let alas_root = as_string(mapping_get(&raw, &["paths", "alas_root"]), "auto");
        let mumu_manager = as_string(mapping_get(&raw, &["paths", "mumu_manager"]), "auto");
        let adb_exe = as_string(mapping_get(&raw, &["paths", "adb_exe"]), "auto");
        let launch_mode = {
            let mode = as_string(mapping_get(&raw, &["paths", "launch_mode"]), "electron").to_ascii_lowercase();
            if mode == "launcher" {
                "launcher".to_string()
            } else {
                "electron".to_string()
            }
        };
        let mumu_instance = match mapping_get(&raw, &["paths", "mumu_instance"]) {
            Some(Value::Number(number)) => number.as_u64().map(|value| value as u32),
            Some(Value::String(text)) => {
                let trimmed = text.trim();
                if trimmed.eq_ignore_ascii_case("auto") {
                    None
                } else {
                    trimmed.parse::<u32>().ok()
                }
            }
            _ => None,
        };
        let config_names = match mapping_get(&raw, &["alas", "config_names"]) {
            Some(Value::Sequence(items)) => {
                let names: Vec<String> = items
                    .iter()
                    .filter_map(|item| item.as_str().map(|text| text.trim().to_string()))
                    .filter(|text| !text.is_empty())
                    .collect();
                if names.is_empty() {
                    None
                } else {
                    Some(names)
                }
            }
            Some(Value::String(text)) => {
                let trimmed = text.trim();
                if trimmed.eq_ignore_ascii_case("auto") || trimmed.is_empty() {
                    None
                } else {
                    Some(
                        trimmed
                            .split(|ch: char| ch == ',' || ch == ';' || ch.is_whitespace())
                            .map(|part| part.trim().to_string())
                            .filter(|part| !part.is_empty())
                            .collect(),
                    )
                }
            }
            _ => None,
        };

        let restart_times = util::parse_hhmm_list(
            mapping_get(&raw, &["schedule", "restart_times"]).unwrap_or(&Value::Null),
        );

        let log_dir_text = as_string(mapping_get(&raw, &["logging", "dir"]), "logs");
        let log_dir_path = {
            let candidate = PathBuf::from(&log_dir_text);
            if candidate.is_absolute() {
                candidate
            } else {
                constants::wd_dir().join(candidate)
            }
        };

        Ok(Settings {
            path: path.to_path_buf(),
            created,
            alas_root,
            mumu_manager,
            mumu_instance,
            adb_exe,
            launch_mode,
            config_names,
            restart_times,
            restart_interval_hours: as_f64(mapping_get(&raw, &["schedule", "restart_interval_hours"]), 6.0).max(0.1),
            restart_on_start: as_bool(mapping_get(&raw, &["schedule", "restart_on_start"]), false),
            ensure_running_on_start: as_bool(mapping_get(&raw, &["schedule", "ensure_running_on_start"]), true),
            retry_delay: as_u64(mapping_get(&raw, &["schedule", "retry_delay_seconds"]), 300).max(30),
            startup_grace_minutes: as_i64(mapping_get(&raw, &["schedule", "startup_grace_minutes"]), 10).max(0),
            check_interval: as_u64(mapping_get(&raw, &["health", "check_interval_seconds"]), 60).max(20),
            adb_fail_threshold: as_u64(mapping_get(&raw, &["health", "adb_fail_threshold"]), 3).max(2) as u32,
            mumu_memory_limit_gb: as_f64(mapping_get(&raw, &["health", "mumu_memory_limit_gb"]), 0.0).max(0.0),
            restart_if_emulator_down: as_bool(mapping_get(&raw, &["health", "restart_if_emulator_down"]), true),
            restart_alas_if_closed: as_bool(mapping_get(&raw, &["health", "restart_alas_if_closed"]), true),
            alas_restart_cooldown: as_i64(mapping_get(&raw, &["health", "alas_restart_cooldown_seconds"]), 600).max(60),
            task_stuck_minutes: as_i64(mapping_get(&raw, &["health", "task_stuck_minutes"]), 60).max(0),
            restart_on_task_stuck: as_bool(mapping_get(&raw, &["health", "restart_on_task_stuck"]), true),
            task_stuck_cooldown: as_i64(mapping_get(&raw, &["health", "task_stuck_cooldown_seconds"]), 3600).max(300),
            emulator_shutdown_timeout: as_u64(mapping_get(&raw, &["timeouts", "emulator_shutdown_seconds"]), 90),
            emulator_boot_timeout: as_u64(mapping_get(&raw, &["timeouts", "emulator_boot_seconds"]), 240),
            adb_ready_timeout: as_u64(mapping_get(&raw, &["timeouts", "adb_ready_seconds"]), 300),
            alas_start_timeout: as_u64(mapping_get(&raw, &["timeouts", "alas_start_seconds"]), 180),
            scheduler_start_timeout: as_u64(mapping_get(&raw, &["timeouts", "scheduler_start_seconds"]), 180),
            kill_adb_server: as_bool(mapping_get(&raw, &["process", "kill_adb_server"]), true),
            use_reloadalas: as_bool(mapping_get(&raw, &["process", "use_reloadalas"]), true),
            kill_alas_timeout: as_u64(mapping_get(&raw, &["process", "kill_alas_seconds"]), 30).max(5),
            status_log_seconds: as_u64(mapping_get(&raw, &["monitor", "status_log_seconds"]), 300).max(30),
            write_status_json: as_bool(mapping_get(&raw, &["monitor", "write_status_json"]), true),
            self_restart_memory_mb: as_u64(mapping_get(&raw, &["monitor", "self_restart_memory_mb"]), 0),
            self_restart_handles: as_u64(mapping_get(&raw, &["monitor", "self_restart_handles"]), 0) as u32,
            self_restart_after_hours: as_u64(mapping_get(&raw, &["monitor", "self_restart_after_hours"]), 0),
            log_dir: log_dir_path,
            keep_days: as_u64(mapping_get(&raw, &["logging", "keep_days"]), 14).max(1) as u32,
            log_level: as_string(mapping_get(&raw, &["logging", "level"]), "INFO").to_ascii_uppercase(),
            mumu_cmd_timeout: as_u64(mapping_get(&raw, &["advanced", "mumu_cmd_timeout_seconds"]), 90).max(20),
        })
    }
}
