//! 状态快照格式化，不负责窗口或控件绘制。
use super::*;
impl GuiApp {
    pub(super) fn status_text(&self, pointer: &str, fallback: &str) -> String {
        self.status
            .pointer(pointer)
            .map(|value| match value {
                Json::String(text) => text.clone(),
                Json::Null => fallback.to_string(),
                other => other.to_string(),
            })
            .unwrap_or_else(|| fallback.to_string())
    }
    pub(super) fn status_line(&self, key: &str) -> String {
        if self.status.is_null() && key != "watchdog" {
            return "正在读取状态…".into();
        }
        match key {
            "watchdog" => {
                let running = util::watcher_running_pid();
                match running {
                    Some(pid) => format!("运行中（PID {pid}）"),
                    None => "未运行".to_string(),
                }
            }
            "next_restart" => self.status_text("/watchdog/next_restart", "尚未安排"),
            "emulator" => {
                let error = self.status_text("/emulator/error", "");
                if !error.is_empty() {
                    return format!("查询失败：{error}");
                }
                let started = self
                    .status
                    .pointer("/emulator/is_process_started")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false)
                    && self
                        .status
                        .pointer("/emulator/is_android_started")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false);
                if !started {
                    return "未运行".to_string();
                }
                format!(
                    "运行中 · {} · PID {} · 内存 {}/{} GB · 已运行 {}",
                    self.status_text("/emulator/state", "?"),
                    self.status_text("/emulator/pid", "?"),
                    self.status_text("/emulator/working_set_gb", "?"),
                    self.status_text("/emulator/commit_gb", "?"),
                    util::fmt_duration(
                        self.status
                            .pointer("/emulator/uptime_seconds")
                            .and_then(|value| value.as_i64())
                            .unwrap_or(0) as f64
                    )
                )
            }
            "adb" => {
                let error = self.status_text("/adb/error", "");
                if !error.is_empty() {
                    return format!("查询失败：{error}");
                }
                format!(
                    "{}（响应 {} ms）",
                    self.status_text("/adb/state", "无响应"),
                    self.status_text("/adb/response_ms", "?")
                )
            }
            "alas" => {
                let error = self.status_text("/alas/error", "");
                if !error.is_empty() {
                    return format!("查询失败：{error}");
                }
                let webui_ok = self
                    .status
                    .pointer("/alas/webui_ok")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                format!(
                    "WebUI {}（{} ms） · 当前任务 {} · Electron PID {}",
                    if webui_ok { "正常" } else { "异常" },
                    self.status_text("/alas/webui_response_ms", "?"),
                    self.status_text("/alas/current_task", "空闲/等待中"),
                    self.status_text("/alas/electron_pid", "-")
                )
            }
            "self" => format!(
                "WS {} MB / Handles {} / CPU {}%（单核）",
                self.status_text("/watchdog/self_working_set_mb", "?"),
                self.status_text("/watchdog/self_handles", "?"),
                self.status_text("/watchdog/self_cpu_pct_one_core", "?")
            ),
            _ => String::new(),
        }
    }
}
