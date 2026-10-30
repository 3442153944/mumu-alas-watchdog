//! 运行状态采集、status.json 与状态日志格式化。

use chrono::{DateTime, Local};
use serde_json::{json, Value as Json};

use crate::alas::Alas;
use crate::config::Settings;
use crate::emulator::Emulator;
use crate::layout::Layout;
use crate::logger;
use crate::logwatch::LogWatcher;
use crate::util;
use crate::win32;

pub fn build_status(
    settings: &Settings,
    layout: &Layout,
    emulator: &Emulator,
    alas: &Alas,
    logwatch: &LogWatcher,
    next_restart: Option<DateTime<Local>>,
    cycle_count: u32,
    failures: u32,
    last_cycle: &Option<Json>,
    started_at: DateTime<Local>,
) -> Json {
    let (working_set, private_mb, handles) = logger::self_metrics();
    let mut status = json!({
        "time": util::now_text(),
        "watchdog": {
            "pid": std::process::id(),
            "version": crate::constants::VERSION,
            "uptime_seconds": (Local::now() - started_at).num_seconds().max(0),
            "next_restart": next_restart.map(|time| time.format("%Y-%m-%d %H:%M:%S").to_string()),
            "cycle_count": cycle_count,
            "failures": failures,
            "last_cycle": last_cycle,
            "self_working_set_mb": (working_set * 100.0).round() / 100.0,
            "self_private_mb": (private_mb * 100.0).round() / 100.0,
            "self_handles": handles,
        },
        "emulator": {},
        "adb": {},
        "alas": {},
    });

    match emulator.info(None) {
        Ok(info) => {
            let pid = info.get("pid").and_then(|value| value.as_u64());
            let (working_set_gb, commit_gb) = pid
                .and_then(|pid| win32::process_memory(pid as u32))
                .map(|(working_set, private)| {
                    (
                        (working_set as f64 / 1024.0 / 1024.0 / 1024.0 * 100.0).round() / 100.0,
                        (private as f64 / 1024.0 / 1024.0 / 1024.0 * 100.0).round() / 100.0,
                    )
                })
                .unwrap_or((0.0, 0.0));
            let started_at = pid
                .and_then(|pid| win32::proc_create_time(pid as u32))
                .map(|time| time.format("%Y-%m-%d %H:%M:%S").to_string());
            let uptime_seconds = pid
                .and_then(|pid| win32::proc_create_time(pid as u32))
                .map(|time| (Local::now() - time).num_seconds().max(0))
                .unwrap_or(0);
            status["emulator"] = json!({
                "name": info.get("name"),
                "index": info.get("index"),
                "state": info.get("player_state"),
                "is_process_started": info.get("is_process_started").and_then(|value| value.as_bool()).unwrap_or(false),
                "is_android_started": info.get("is_android_started").and_then(|value| value.as_bool()).unwrap_or(false),
                "pid": pid,
                "adb_port": info.get("adb_port"),
                "working_set_gb": working_set_gb,
                "commit_gb": commit_gb,
                "started_at": started_at,
                "uptime_seconds": uptime_seconds,
            });
        }
        Err(err) => {
            status["emulator"] = json!({"error": err});
        }
    }

    let (adb_state, adb_ms) = emulator.timed_adb_state();
    status["adb"] = json!({
        "serial": emulator.serial,
        "state": adb_state,
        "response_ms": adb_ms,
    });

    let main_pid = alas.electron_main_pid();
    let (webui_ok, webui_ms) = alas.webui_status();
    status["alas"] = json!({
        "electron_pid": main_pid,
        "electron_processes": alas.electron_process_count(),
        "webui_ok": webui_ok,
        "webui_response_ms": webui_ms,
        "webui_port": layout.webui_port,
        "task_info": logwatch.status_dict(),
        "current_task": logwatch.status_dict().get("current_task").cloned().unwrap_or(Json::Null),
        "manual_stopped": logwatch.manual_stopped,
    });

    let _ = settings;
    status
}

pub fn write_status(settings: &Settings, status: &Json) {
    if settings.write_status_json {
        util::atomic_write_json(&crate::constants::status_json(), status);
    }
}

pub fn format_lines(status: &Json, next_restart: Option<DateTime<Local>>) -> Vec<String> {
    let mut lines = Vec::new();
    let emulator = &status["emulator"];
    if let Some(error) = emulator.get("error").and_then(|value| value.as_str()) {
        lines.push(format!("[状态] 模拟器查询失败: {error}"));
    } else {
        lines.push(format!(
            "[状态] 模拟器: state={} pid={} 工作集={}GB 提交={}GB 已运行={}",
            emulator
                .get("state")
                .and_then(|value| value.as_str())
                .unwrap_or("?"),
            emulator
                .get("pid")
                .and_then(|value| value.as_u64())
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "?".to_string()),
            emulator
                .get("working_set_gb")
                .and_then(|value| value.as_f64())
                .unwrap_or(0.0),
            emulator
                .get("commit_gb")
                .and_then(|value| value.as_f64())
                .unwrap_or(0.0),
            util::fmt_duration(
                emulator
                    .get("uptime_seconds")
                    .and_then(|value| value.as_i64())
                    .unwrap_or(0) as f64
            )
        ));
    }
    let adb = &status["adb"];
    let alas = &status["alas"];
    lines.push(format!(
        "[状态] ADB: {}（{} ms） | WebUI: {}（{} ms）",
        adb.get("state")
            .and_then(|value| value.as_str())
            .filter(|text| !text.is_empty())
            .unwrap_or("无响应"),
        adb.get("response_ms")
            .and_then(|value| value.as_u64())
            .unwrap_or(0),
        if alas
            .get("webui_ok")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            "正常"
        } else {
            "异常"
        },
        alas.get("webui_response_ms")
            .and_then(|value| value.as_u64())
            .unwrap_or(0),
    ));
    let task = alas
        .get("current_task")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    if task.is_empty() {
        lines.push(format!(
            "[状态] Alas: 空闲/等待任务 | Electron PID={}",
            alas.get("electron_pid")
                .and_then(|value| value.as_u64())
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "?".to_string())
        ));
    } else {
        lines.push(format!(
            "[状态] Alas: 当前任务 {task}，最近日志 {}",
            alas.pointer("/task_info/last_activity")
                .and_then(|value| value.as_str())
                .unwrap_or("?")
        ));
    }
    let self_ws = status
        .pointer("/watchdog/self_working_set_mb")
        .and_then(|value| value.as_f64())
        .unwrap_or(0.0);
    let self_handles = status
        .pointer("/watchdog/self_handles")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    let self_cpu = status
        .pointer("/watchdog/self_cpu_pct_one_core")
        .and_then(|value| value.as_f64());
    let cpu_text = self_cpu
        .map(|pct| format!(" / CPU {pct:.2}%"))
        .unwrap_or_default();
    lines.push(format!(
        "[状态] 看门狗自身: WS {self_ws:.1} MB / Handles {self_handles}{cpu_text} | 计划: {}",
        next_restart
            .map(|time| time.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "尚未安排".to_string())
    ));
    lines
}
