//! 全局常量与运行路径（源码 / 发布 exe 均适用）。

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const APP_NAME: &str = "MuMuAlasWatchdog";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const AUTOSTART_SHORTCUT_NAME: &str = "MuMuAlasWatchdog.lnk";
pub const SUPERVISOR_TASK_NAME: &str = "MuMuAlasWatchdog-Supervisor";

pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
pub const DETACHED_PROCESS: u32 = 0x0000_0008;
pub const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

/// 进程环境变量黑名单：这些会破坏 Electron / PyInstaller 或干扰子进程。
pub const BLOCKED_ENV_KEYS: &[&str] = &[
    "ELECTRON_RUN_AS_NODE",
    "ELECTRON_NO_ATTACH_CONSOLE",
    "MUW_CMD_LOG",
];
pub const BLOCKED_ENV_PREFIXES: &[&str] = &["_PYI_", "PYINSTALLER_"];

/// 需要产生告警（但不自动处理）的 Alas 日志片段。
pub const FATAL_LOG_PATTERNS: &[&str] = &[
    "Request human takeover",
    "RequestHumanTakeover",
    "GamePageUnknownError",
    "failed 3 or more times",
    "crashed",
    "ScriptError",
];

static WD_DIR_CELL: OnceLock<PathBuf> = OnceLock::new();
static APP_PATH_CELL: OnceLock<PathBuf> = OnceLock::new();

/// 程序所在目录：发布版为 exe 同目录；开发版（target/debug|release）回落到 crate 根目录。
pub fn wd_dir() -> &'static Path {
    WD_DIR_CELL.get_or_init(|| {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                let normalized = parent.to_string_lossy().replace('/', "\\").to_ascii_lowercase();
                if normalized.contains("\\target\\debug") || normalized.contains("\\target\\release") {
                    return PathBuf::from(env!("CARGO_MANIFEST_DIR"));
                }
                return parent.to_path_buf();
            }
        }
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
    })
}

/// 可执行文件路径（发布版为 exe，开发版为 target 下的二进制）。
pub fn app_path() -> &'static Path {
    APP_PATH_CELL.get_or_init(|| {
        std::env::current_exe().unwrap_or_else(|_| PathBuf::from("MuMuAlasWatchdog.exe"))
    })
}

pub fn config_path() -> PathBuf {
    wd_dir().join("config.yml")
}

pub fn config_example_path() -> PathBuf {
    wd_dir().join("config.example.yml")
}

pub fn pid_file() -> PathBuf {
    wd_dir().join("watchdog.pid")
}

pub fn status_json() -> PathBuf {
    wd_dir().join("status.json")
}

pub fn flag_full_restart() -> PathBuf {
    wd_dir().join("restart_now.flag")
}

pub fn flag_alas_restart() -> PathBuf {
    wd_dir().join("restart_alas_now.flag")
}

pub fn flag_stop() -> PathBuf {
    wd_dir().join("stop.flag")
}
