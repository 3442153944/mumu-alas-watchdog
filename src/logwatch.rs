//! Alas 日志增量读取、任务状态跟踪、调度器启动等待。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use serde_json::{json, Value as Json};

use crate::config::Settings;
use crate::constants;
use crate::layout::Layout;
use crate::util;

#[derive(Debug, Clone, Default)]
pub struct TaskState {
    pub current_task: Option<String>,
    pub task_started: Option<DateTime<Local>>,
    pub last_activity: Option<DateTime<Local>>,
    pub last_log_file: Option<String>,
    pub last_line: String,
}

pub struct LogWatcher {
    log_dir: PathBuf,
    configs: Vec<String>,
    offsets: HashMap<PathBuf, u64>,
    pub task_state: HashMap<String, TaskState>,
    pub manual_stopped: bool,
    manual_warned: bool,
    seen_keys: HashSet<(String, String)>,
    stuck_keys: HashSet<(String, String, Option<String>)>,
}

fn parse_log_time(line: &str) -> Option<DateTime<Local>> {
    if line.len() < 23 {
        return None;
    }
    NaiveDateTime::parse_from_str(&line[..23], "%Y-%m-%d %H:%M:%S%.3f")
        .ok()
        .and_then(|naive| naive.and_local_timezone(Local).single())
}

impl LogWatcher {
    pub fn new(_settings: &Settings, layout: &Layout) -> Self {
        let mut watcher = LogWatcher {
            log_dir: layout.alas_log_dir.clone(),
            configs: layout.configs.clone(),
            offsets: HashMap::new(),
            task_state: HashMap::new(),
            manual_stopped: false,
            manual_warned: false,
            seen_keys: HashSet::new(),
            stuck_keys: HashSet::new(),
        };
        for config in &watcher.configs {
            watcher.task_state.insert(config.clone(), TaskState::default());
        }
        watcher.bootstrap();
        watcher.prime_offsets();
        watcher
    }

    fn log_files(&self) -> Vec<PathBuf> {
        let Ok(entries) = fs::read_dir(&self.log_dir) else {
            return Vec::new();
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("txt"))
            .collect();
        files.sort();
        files
    }

    pub fn prime_offsets(&mut self) {
        self.offsets.clear();
        for path in self.log_files() {
            if let Ok(metadata) = fs::metadata(&path) {
                self.offsets.insert(path, metadata.len());
            }
        }
    }

    pub fn offsets_snapshot(&self) -> HashMap<PathBuf, u64> {
        self.offsets.clone()
    }

    pub fn restore_offsets(&mut self, offsets: HashMap<PathBuf, u64>) {
        self.offsets = offsets;
    }

    pub fn scan(&mut self) -> HashMap<String, String> {
        let files = self.log_files();
        util::read_appended(&files, &mut self.offsets)
    }

    fn bootstrap(&mut self) {
        for config in self.configs.clone() {
            let prefix = format!("_{config}.txt");
            let mut candidates: Vec<PathBuf> = self
                .log_files()
                .into_iter()
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .map(|name| name.ends_with(&prefix))
                        .unwrap_or(false)
                })
                .collect();
            candidates.sort_by_key(|path| {
                fs::metadata(path)
                    .and_then(|metadata| metadata.modified())
                    .unwrap_or(std::time::UNIX_EPOCH)
            });
            let Some(path) = candidates.last() else {
                continue;
            };
            let mtime = fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_secs() as i64)
                .and_then(|seconds| Local.timestamp_opt(seconds, 0).single())
                .unwrap_or_else(Local::now);
            let state = self.task_state.entry(config).or_default();
            state.last_activity = Some(mtime);
            state.last_log_file = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.to_string());
            for line in util::tail_lines(path, 5000, 8 * 1024 * 1024) {
                if let Some(task) = extract_task_start(&line) {
                    state.current_task = Some(task);
                    state.task_started = parse_log_time(&line).or(Some(mtime));
                }
                if line.contains("Scheduler: End task `") {
                    state.current_task = None;
                    state.task_started = None;
                }
                if !line.trim().is_empty() {
                    state.last_line = line.trim().chars().take(200).collect();
                }
            }
        }
    }

    pub fn analyze(&mut self, texts: &HashMap<String, String>) {
        let now = Local::now();
        for (file_name, text) in texts {
            if text.is_empty() {
                continue;
            }
            for config in self.configs.clone() {
                if file_name.ends_with(&format!("_{config}.txt")) {
                    let state = self.task_state.entry(config).or_default();
                    for line in text.lines() {
                        if let Some(task) = extract_task_start(line) {
                            state.current_task = Some(task);
                            state.task_started = parse_log_time(line).or(Some(now));
                        }
                        if line.contains("Scheduler: End task `") {
                            state.current_task = None;
                            state.task_started = None;
                        }
                        if !line.trim().is_empty() {
                            state.last_activity = Some(now);
                            state.last_line = line.trim().chars().take(200).collect();
                        }
                    }
                    state.last_log_file = Some(file_name.clone());
                }
            }
            for pattern in constants::FATAL_LOG_PATTERNS {
                if !text.contains(pattern) {
                    continue;
                }
                for line in text.lines() {
                    if !line.contains(pattern) {
                        continue;
                    }
                    let key = (pattern.to_string(), line.trim().chars().take(160).collect());
                    if self.seen_keys.contains(&key) {
                        continue;
                    }
                    if self.seen_keys.len() > 2000 {
                        self.seen_keys.clear();
                    }
                    self.seen_keys.insert(key);
                    log::warn!(
                        "[Alas] 日志告警 [{file_name}]: {}",
                        line.trim().chars().take(180).collect::<String>()
                    );
                }
            }
            for config in self.configs.clone() {
                if text.contains(&format!("Start scheduler loop: {config}")) {
                    self.manual_stopped = false;
                    self.manual_warned = false;
                }
                if text.contains(&format!("[{config}] exited")) && !self.manual_warned {
                    self.manual_stopped = true;
                    self.manual_warned = true;
                    log::info!(
                        "[Alas] 检测到调度器被手动停止，自动恢复已暂停（下次计划重启或 --restart-alas-now 会恢复）"
                    );
                }
            }
            if text.contains("Alas closed.") && !self.manual_warned {
                self.manual_stopped = true;
                self.manual_warned = true;
                log::info!("[Alas] 检测到 Alas 被手动关闭，自动恢复已暂停（下次计划重启会自动恢复）");
            }
        }
    }

    pub fn note_scheduler_started(&mut self) {
        self.manual_stopped = false;
        self.manual_warned = false;
    }

    /// 等待所有配置出现 Start scheduler loop 标记；offsets 会被推进。
    pub fn wait_scheduler_started(
        &mut self,
        offsets: &mut HashMap<PathBuf, u64>,
        electron_alive: &dyn Fn() -> bool,
        webui_ok: &dyn Fn() -> bool,
        timeout_seconds: u64,
    ) -> Result<(), String> {
        log::info!("[Alas] 等待游戏脚本（调度器）自动启动...");
        let mut pending: Vec<String> = self.configs.clone();
        let deadline = Instant::now() + Duration::from_secs(timeout_seconds);
        while !pending.is_empty() && Instant::now() < deadline {
            let files = self.log_files();
            let texts = util::read_appended(&files, offsets);
            for text in texts.values() {
                pending.retain(|config| !text.contains(&format!("Start scheduler loop: {config}")));
            }
            if pending.is_empty() {
                break;
            }
            if !electron_alive() && !webui_ok() {
                return Err("Alas 在启动过程中意外退出".to_string());
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        if !pending.is_empty() {
            return Err(format!(
                "等待调度器自动启动超时（{timeout_seconds} 秒），未完成: {pending:?}"
            ));
        }
        self.note_scheduler_started();
        Ok(())
    }

    pub fn last_start_line(&self) -> Option<(String, String)> {
        let mut files = self.log_files();
        files.sort_by_key(|path| {
            fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .unwrap_or(std::time::UNIX_EPOCH)
        });
        for path in files.iter().rev() {
            if let Some(line) = util::last_line_with(path, "Start scheduler loop:") {
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
                    .to_string();
                return Some((name, line));
            }
        }
        None
    }

    pub fn current_task(&self) -> Option<(&String, &TaskState)> {
        self.task_state
            .iter()
            .find(|(_, state)| state.current_task.is_some())
    }

    pub fn status_dict(&self) -> Json {
        if let Some((config, state)) = self.current_task() {
            json!({
                "config": config,
                "current_task": state.current_task,
                "task_started": state.task_started.map(|time| time.format("%Y-%m-%d %H:%M:%S").to_string()),
                "last_activity": state.last_activity.map(|time| time.format("%Y-%m-%d %H:%M:%S").to_string()),
                "manual_stopped": self.manual_stopped,
            })
        } else {
            json!({"current_task": null, "manual_stopped": self.manual_stopped})
        }
    }

    /// 记录一次卡死判断的键，返回 true 表示首次发现。
    pub fn mark_stuck_once(&mut self, config: &str, task: &str, started: Option<&DateTime<Local>>) -> bool {
        let key = (
            config.to_string(),
            task.to_string(),
            started.map(|time| time.format("%Y-%m-%d %H:%M:%S").to_string()),
        );
        if self.stuck_keys.contains(&key) {
            false
        } else {
            self.stuck_keys.insert(key);
            true
        }
    }

    pub fn log_dir(&self) -> &Path {
        &self.log_dir
    }
}

fn extract_task_start(line: &str) -> Option<String> {
    let marker = "Scheduler: Start task `";
    let start = line.find(marker)? + marker.len();
    let rest = &line[start..];
    let end = rest.find('`')?;
    Some(rest[..end].to_string())
}
