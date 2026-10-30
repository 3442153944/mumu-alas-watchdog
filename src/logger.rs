//! 轻量日志：按天滚动文件 + 可选控制台输出；不依赖第三方日志框架。

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::Local;
use log::{Level, LevelFilter, Log, Metadata, Record};

use crate::constants;
use crate::util;
use crate::win32;

struct LoggerState {
    file: Option<File>,
    date: String,
    dir: PathBuf,
    level: LevelFilter,
    keep_days: u32,
}

pub struct FileLogger {
    console: bool,
    state: Mutex<LoggerState>,
}

impl FileLogger {
    fn new(dir: &Path, level: LevelFilter, console: bool) -> Self {
        let _ = fs::create_dir_all(dir);
        util::prune_logs(dir, 14);
        FileLogger {
            console,
            state: Mutex::new(LoggerState {
                file: None,
                date: String::new(),
                dir: dir.into(),
                level,
                keep_days: 14,
            }),
        }
    }
}

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        self.state
            .lock()
            .map(|state| metadata.level() <= state.level)
            .unwrap_or(false)
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S");
        let line = format!("{timestamp} | {:5} | {}", record.level(), record.args());
        if self.console {
            // 控制台可用时才写；窗口化 exe 无控制台时静默。
            let mut stdout = std::io::stdout();
            let _ = writeln!(stdout, "{line}");
        }
        let today = Local::now().format("%Y%m%d").to_string();
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        if state.date != today || state.file.is_none() {
            state.date = today.clone();
            let path = state.dir.join(format!("watchdog_{today}.log"));
            state.file = OpenOptions::new().create(true).append(true).open(path).ok();
            util::prune_logs(&state.dir, state.keep_days);
        }
        if let Some(file) = state.file.as_mut() {
            let _ = writeln!(file, "{line}");
            let _ = file.flush();
        }
    }

    fn flush(&self) {}
}

static LOGGER: std::sync::OnceLock<FileLogger> = std::sync::OnceLock::new();

/// Prepare the destination before replacing any live logger state.
pub fn reconfigure(dir: &Path, level: &str, keep_days: u32) -> Result<(), String> {
    let level = match level.to_ascii_uppercase().as_str() {
        "DEBUG" => LevelFilter::Debug,
        "WARN" | "WARNING" => LevelFilter::Warn,
        "ERROR" => LevelFilter::Error,
        _ => LevelFilter::Info,
    };
    fs::create_dir_all(dir).map_err(|e| format!("日志目录不可用：{e}"))?;
    let date = Local::now().format("%Y%m%d").to_string();
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(format!("watchdog_{date}.log")))
        .map_err(|e| format!("日志文件不可写：{e}"))?;
    if let Some(logger) = LOGGER.get() {
        let mut state = logger.state.lock().map_err(|e| e.to_string())?;
        *state = LoggerState {
            file: Some(file),
            date,
            dir: dir.into(),
            level,
            keep_days,
        };
        log::set_max_level(level);
    }
    Ok(())
}

pub fn init(dir: &Path, level: &str, console: bool) {
    let level_filter = match level.to_ascii_uppercase().as_str() {
        "DEBUG" => LevelFilter::Debug,
        "WARN" | "WARNING" => LevelFilter::Warn,
        "ERROR" => LevelFilter::Error,
        _ => LevelFilter::Info,
    };
    // 窗口化/无控制台时关闭 console 输出，避免写入无效句柄。
    let console_enabled = console && win32::console_available();
    let logger = FileLogger::new(dir, level_filter, console_enabled);
    let _ = LOGGER.set(logger);
    if let Some(logger) = LOGGER.get() {
        let _ = log::set_logger(logger);
        log::set_max_level(level_filter);
    }
    if console_enabled {
        log::debug!("console logging enabled");
    }
    util::install_panic_hook();
    let _ = constants::APP_NAME;
}

/// 当前进程基础指标：(工作集 MB, 提交 MB, 句柄数)
pub fn self_metrics() -> (f64, f64, u32) {
    let (working_set, private) = win32::self_memory();
    (
        working_set as f64 / 1024.0 / 1024.0,
        private as f64 / 1024.0 / 1024.0,
        win32::self_handle_count(),
    )
}

pub fn level_name(level: Level) -> &'static str {
    match level {
        Level::Error => "ERROR",
        Level::Warn => "WARN",
        Level::Info => "INFO",
        Level::Debug => "DEBUG",
        Level::Trace => "TRACE",
    }
}
