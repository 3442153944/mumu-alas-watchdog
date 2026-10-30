//! 通用工具：时间/路径、命令执行（带超时）、HTTP 探测、日志尾部读取、JSON 原子写入。

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};

use crate::constants;
use crate::win32;

/// 当前环境变量（过滤掉会破坏 Electron/PyInstaller 或干扰子进程的键）。
pub fn clean_env() -> Vec<(OsString, OsString)> {
    let blocked_prefixes = constants::BLOCKED_ENV_PREFIXES;
    std::env::vars_os()
        .filter(|(key, _)| {
            let key_text = key.to_string_lossy().to_ascii_uppercase();
            if constants::BLOCKED_ENV_KEYS
                .iter()
                .any(|blocked| key_text == *blocked)
            {
                return false;
            }
            !blocked_prefixes
                .iter()
                .any(|prefix| key_text.starts_with(prefix))
        })
        .collect()
}

/// 用 Win32 CreateProcess（经 std）启动进程，可选清洗环境、隐藏窗口、分离运行。
pub fn spawn_process(
    exe: &Path,
    args: &[String],
    cwd: Option<&Path>,
    env_clean: bool,
    hidden: bool,
    detached: bool,
) -> std::io::Result<u32> {
    let mut command = Command::new(exe);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    if env_clean {
        command.env_clear();
        for (key, value) in clean_env() {
            command.env(key, value);
        }
    }
    let mut flags = 0u32;
    if hidden {
        flags |= constants::CREATE_NO_WINDOW;
    }
    if detached {
        flags |= constants::DETACHED_PROCESS | constants::CREATE_NEW_PROCESS_GROUP;
    }
    if flags != 0 {
        command.creation_flags(flags);
    }
    command.spawn().map(|child| child.id())
}

/// 等待 PID 文件出现并返回其中 PID（最多等待 timeout）。
pub fn wait_pid_file(path: &Path, timeout: Duration) -> Option<u32> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if let Ok(text) = fs::read_to_string(path) {
            if let Ok(pid) = text.trim().parse::<u32>() {
                if win32::is_pid_alive(pid) {
                    return Some(pid);
                }
            }
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    None
}

#[derive(Debug, Clone)]
pub struct CmdResult {
    pub rc: i32,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

impl CmdResult {
    pub fn combined(&self) -> String {
        let out = self.stdout.trim();
        if !out.is_empty() {
            out.to_string()
        } else {
            self.stderr.trim().to_string()
        }
    }
}

/// 执行命令并捕获输出；带超时（超时后 kill）。std 的 Command 在 Windows 下最终走 CreateProcessW。
pub fn run_cmd<S: AsRef<std::ffi::OsStr>>(program: S, args: &[&str], timeout: Duration) -> CmdResult {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(constants::CREATE_NO_WINDOW);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => {
            return CmdResult {
                rc: -2,
                stdout: String::new(),
                stderr: err.to_string(),
                timed_out: false,
            }
        }
    };
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    let output = drain_output(&mut child);
                    return CmdResult {
                        rc: -1,
                        stdout: output.0,
                        stderr: output.1,
                        timed_out: true,
                    };
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(err) => {
                return CmdResult {
                    rc: -2,
                    stdout: String::new(),
                    stderr: err.to_string(),
                    timed_out: false,
                }
            }
        }
    }
    let output = drain_output(&mut child);
    CmdResult {
        rc: child.wait().map(|status| status.code().unwrap_or(-1)).unwrap_or(-1),
        stdout: output.0,
        stderr: output.1,
        timed_out: false,
    }
}

fn drain_output(child: &mut std::process::Child) -> (String, String) {
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    (stdout, stderr)
}

/// 便捷执行：直接传入完整参数列表（第一个元素是程序）。
pub fn run_command_line(command_line: &[String], timeout: Duration) -> CmdResult {
    if command_line.is_empty() {
        return CmdResult {
            rc: -2,
            stdout: String::new(),
            stderr: "empty command".to_string(),
            timed_out: false,
        };
    }
    let args: Vec<&str> = command_line.iter().skip(1).map(|item| item.as_str()).collect();
    run_cmd(&command_line[0], &args, timeout)
}

pub fn taskkill_tree(pid: u32) -> bool {
    let pid_text = pid.to_string();
    let result = run_cmd(
        "taskkill",
        &["/F", "/T", "/PID", pid_text.as_str()],
        Duration::from_secs(30),
    );
    result.rc == 0
}

pub fn now() -> DateTime<Local> {
    Local::now()
}

pub fn now_text() -> String {
    now().format("%Y-%m-%d %H:%M:%S").to_string()
}

pub fn fmt_duration(seconds: f64) -> String {
    let total = seconds.max(0.0) as i64;
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let secs = total % 60;
    if hours > 0 {
        format!("{}小时{}分{}秒", hours, minutes, secs)
    } else if minutes > 0 {
        format!("{}分{}秒", minutes, secs)
    } else {
        format!("{}秒", secs)
    }
}

pub fn norm_path(path: &Path) -> String {
    fn normalize_text(text: &str) -> String {
        let without_verbatim = text.strip_prefix(r"\\?\").unwrap_or(text);
        without_verbatim.replace('/', "\\").to_ascii_lowercase()
    }
    match path.canonicalize() {
        Ok(resolved) => normalize_text(&resolved.to_string_lossy()),
        Err(_) => normalize_text(&path.to_string_lossy()),
    }
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    norm_path(a) == norm_path(b)
}

pub fn path_is_under(child: &Path, base: &Path) -> bool {
    let child_s = norm_path(child);
    let base_s = norm_path(base);
    child_s == base_s
        || child_s
            .strip_prefix(&base_s)
            .map(|rest| rest.starts_with('\\'))
            .unwrap_or(false)
}

/// 去掉 Windows canonicalize 产生的 `\\?\` 前缀，便于显示与传参。
pub fn strip_verbatim(path: &Path) -> std::path::PathBuf {
    let text = path.to_string_lossy();
    let cleaned = text.strip_prefix(r"\\?\").unwrap_or(&text);
    std::path::PathBuf::from(cleaned.to_string())
}

pub fn port_listening(port: u16, timeout: Duration) -> bool {
    use std::net::TcpStream;
    let address = format!("127.0.0.1:{}", port);
    address
        .parse::<std::net::SocketAddr>()
        .ok()
        .and_then(|socket| TcpStream::connect_timeout(&socket, timeout).ok())
        .is_some()
}

/// 返回 (WebUI 是否可访问, 响应毫秒)。
pub fn http_probe(port: u16, timeout: Duration) -> (bool, u128) {
    let started = Instant::now();
    let url = format!("http://127.0.0.1:{}/", port);
    let ok = match ureq::AgentBuilder::new().timeout(timeout).build().get(&url).call() {
        Ok(response) => (200..500).contains(&response.status()),
        Err(_) => false,
    };
    (ok, started.elapsed().as_millis())
}

pub fn parse_hhmm_list(value: &serde_yaml::Value) -> Vec<(u32, u32)> {
    let mut items: Vec<String> = Vec::new();
    match value {
        serde_yaml::Value::String(text) => {
            items = text
                .split(|ch: char| ch == ',' || ch == ';' || ch.is_whitespace())
                .map(|part| part.trim().trim_matches('"').trim_matches('\'').to_string())
                .collect();
        }
        serde_yaml::Value::Sequence(sequence) => {
            for item in sequence {
                if let serde_yaml::Value::String(text) = item {
                    items.push(text.trim().to_string());
                }
            }
        }
        _ => {}
    }
    let mut result = Vec::new();
    for item in items {
        let mut parts = item.splitn(2, ':');
        let (Some(hour), Some(minute)) = (parts.next(), parts.next()) else {
            continue;
        };
        let (Ok(hour), Ok(minute)) = (hour.parse::<u32>(), minute.parse::<u32>()) else {
            continue;
        };
        if hour < 24 && minute < 60 {
            result.push((hour, minute));
        }
    }
    result.sort_unstable();
    result.dedup();
    result
}

pub fn tail_lines(path: &Path, max_lines: usize, max_bytes: usize) -> Vec<String> {
    let Ok(metadata) = fs::metadata(path) else {
        return Vec::new();
    };
    let file_size = metadata.len() as usize;
    let start = file_size.saturating_sub(max_bytes);
    let Ok(mut file) = fs::File::open(path) else {
        return Vec::new();
    };
    if start > 0 {
        use std::io::{Seek, SeekFrom};
        let _ = file.seek(SeekFrom::Start(start as u64));
    }
    let mut buffer = Vec::with_capacity(file_size.saturating_sub(start));
    let _ = file.read_to_end(&mut buffer);
    let text = String::from_utf8_lossy(&buffer);
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(max_lines)..]
        .iter()
        .map(|line| line.to_string())
        .collect()
}

pub fn last_line_with(path: &Path, needle: &str) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    text.lines()
        .rev()
        .find(|line| line.contains(needle))
        .map(|line| line.trim().to_string())
}

pub fn atomic_write_json(path: &Path, value: &serde_json::Value) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let temp = path.with_extension("json.tmp");
    if let Ok(text) = serde_json::to_string_pretty(value) {
        if fs::write(&temp, text).is_ok() {
            let _ = fs::rename(&temp, path);
        }
    }
}

pub fn is_pid_alive(pid: u32) -> bool {
    crate::win32::is_pid_alive(pid)
}

pub fn watcher_running_pid() -> Option<u32> {
    let path = constants::pid_file();
    let text = fs::read_to_string(path).ok()?;
    let pid = text.trim().parse::<u32>().ok()?;
    if is_pid_alive(pid) {
        Some(pid)
    } else {
        None
    }
}

/// 读取版本信息、清理测试用。
pub fn read_json(path: &Path) -> Option<serde_json::Value> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// 保留旧日志：删除 keep_days 之前按日期命名的日志。
pub fn prune_logs(log_dir: &Path, keep_days: u32) {
    let Ok(entries) = fs::read_dir(log_dir) else {
        return;
    };
    let cutoff = now() - chrono::Duration::days(keep_days.max(1) as i64);
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
        if !name.starts_with("watchdog_") || !name.ends_with(".log") {
            continue;
        }
        let Some(date_text) = name
            .strip_prefix("watchdog_")
            .and_then(|rest| rest.strip_suffix(".log"))
        else {
            continue;
        };
        let Ok(date) = chrono::NaiveDate::parse_from_str(date_text, "%Y%m%d") else {
            continue;
        };
        if date < cutoff.date_naive() {
            let _ = fs::remove_file(&path);
        }
    }
}

pub fn newest_log_file(log_dir: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(log_dir).ok()?;
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().and_then(|name| name.to_str()).unwrap_or_default();
        if !name.starts_with("watchdog_") || !name.ends_with(".log") {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        if best.as_ref().map(|(time, _)| modified > *time).unwrap_or(true) {
            best = Some((modified, path));
        }
    }
    best.map(|(_, path)| path)
}

/// 读取 Alas 日志目录的追加内容（文件名 -> 新增文本）。
pub fn read_appended(
    log_files: &[PathBuf],
    offsets: &mut HashMap<PathBuf, u64>,
) -> HashMap<String, String> {
    let mut result = HashMap::new();
    for path in log_files {
        let Ok(metadata) = fs::metadata(path) else {
            continue;
        };
        let size = metadata.len();
        let base = offsets.get(path).copied().unwrap_or(0);
        let base = if size < base { 0 } else { base };
        if size == base {
            offsets.insert(path.clone(), size);
            continue;
        }
        let Ok(mut file) = fs::File::open(path) else {
            continue;
        };
        use std::io::{Seek, SeekFrom};
        if base > 0 {
            let _ = file.seek(SeekFrom::Start(base));
        }
        let mut buffer = Vec::new();
        let _ = file.read_to_end(&mut buffer);
        offsets.insert(path.clone(), size);
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        result.insert(name, String::from_utf8_lossy(&buffer).to_string());
    }
    result
}

pub fn check_output(program: &str, args: &[&str]) -> Option<Output> {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(constants::CREATE_NO_WINDOW)
        .output()
        .ok()
}

/// 把 panic 原因和后调用栈写入 crash.log：
/// 窗口化进程没有控制台，否则异常退出的原因会完全丢失。
pub fn install_panic_hook() {
    use std::sync::OnceLock;
    static INSTALLED: OnceLock<()> = OnceLock::new();
    if INSTALLED.set(()).is_err() {
        return;
    }
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = format!("panic: {info}");
        log::error!("{message}");
        let backtrace = std::backtrace::Backtrace::force_capture();
        let crash_path = crate::constants::wd_dir().join("crash.log");
        let _ = std::fs::write(
            &crash_path,
            format!("{}\n{message}\n{backtrace}\n", now_text()),
        );
        default_hook(info);
    }));
}
