//! MuMu 模拟器控制：MuMuManager CLI、启停、ADB 连接等待。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::Value as Json;

use crate::config::Settings;
use crate::layout::Layout;
use crate::util;
use crate::win32;

pub struct Emulator {
    settings: Settings,
    pub manager: PathBuf,
    pub adb_exe: PathBuf,
    pub instance: u32,
    pub serial: String,
    pub last_pid: Option<u32>,
}

pub fn query_info_all(manager: &Path, timeout: Duration) -> std::collections::HashMap<String, Json> {
    let result = util::run_cmd(manager, &["info", "-v", "all"], timeout);
    let text = result.combined();
    if !text.trim_start().starts_with('{') {
        return std::collections::HashMap::new();
    }
    match serde_json::from_str::<Json>(&text) {
        Ok(Json::Object(map)) => map.into_iter().collect(),
        _ => std::collections::HashMap::new(),
    }
}

impl Emulator {
    pub fn new(settings: &Settings, layout: &Layout) -> Self {
        Emulator {
            settings: settings.clone(),
            manager: layout.mumu_manager.clone(),
            adb_exe: layout.adb_exe.clone(),
            instance: layout.instance,
            serial: layout.serial.clone(),
            last_pid: None,
        }
    }

    pub fn raw(&self, args: &[&str], timeout: Duration) -> Result<Json, String> {
        let result = util::run_cmd(&self.manager, args, timeout);
        let text = result.combined();
        if result.timed_out {
            return Err(format!("MuMuManager 命令超时: {:?}", args));
        }
        let trimmed = text.trim();
        if trimmed.starts_with('{') {
            if let Ok(Json::Object(map)) = serde_json::from_str::<Json>(trimmed) {
                let code = map
                    .get("errcode")
                    .or_else(|| map.get("error_code"))
                    .and_then(|value| value.as_i64());
                if let Some(code) = code {
                    if code != 0 {
                        return Err(format!("MuMuManager 返回错误: {trimmed}"));
                    }
                }
                return Ok(Json::Object(map));
            }
        }
        if result.rc != 0 {
            return Err(format!("MuMuManager 命令失败 rc={}: {}", result.rc, trimmed));
        }
        Ok(Json::String(trimmed.to_string()))
    }

    pub fn info(&self, index: Option<u32>) -> Result<Json, String> {
        let idx = index.unwrap_or(self.instance);
        let data = self.raw(&["info", "-v", &idx.to_string()], Duration::from_secs(25))?;
        if !data.is_object() || data.get("index").is_none() {
            return Err(format!("无法解析 MuMu 实例 {idx} 的信息: {data}"));
        }
        Ok(data)
    }

    pub fn info_all(&self) -> std::collections::HashMap<String, Json> {
        query_info_all(&self.manager, Duration::from_secs(25))
    }

    pub fn is_running(info: &Json) -> bool {
        let started = info
            .get("is_process_started")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let android = info
            .get("is_android_started")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        started && android
    }

    pub fn refresh(&mut self) -> Result<Json, String> {
        let info = self.info(None)?;
        self.last_pid = info
            .get("pid")
            .and_then(|value| value.as_u64())
            .map(|pid| pid as u32);
        if let Some(port) = info.get("adb_port").and_then(|value| value.as_u64()) {
            let host = info
                .get("adb_host_ip")
                .and_then(|value| value.as_str())
                .unwrap_or("127.0.0.1");
            self.serial = format!("{}:{}", host, port);
        }
        Ok(info)
    }

    pub fn adb(&self, args: &[&str], timeout: Duration) -> util::CmdResult {
        util::run_cmd(&self.adb_exe, args, timeout)
    }

    pub fn adb_state(&self) -> String {
        let result = self.adb(&["-s", &self.serial, "get-state"], Duration::from_secs(15));
        let text = if result.stdout.trim().is_empty() {
            result.stderr.trim().to_string()
        } else {
            result.stdout.trim().to_lowercase()
        };
        text.trim().to_lowercase()
    }

    pub fn timed_adb_state(&self) -> (String, u128) {
        let started = Instant::now();
        let state = self.adb_state();
        (state, started.elapsed().as_millis())
    }

    pub fn adb_shell(&self, args: &[&str], timeout: Duration) -> util::CmdResult {
        let mut full: Vec<&str> = vec!["-s", &self.serial, "shell"];
        full.extend_from_slice(args);
        self.adb(&full, timeout)
    }

    pub fn kill_adb_server(&self) {
        let result = self.adb(&["kill-server"], Duration::from_secs(30));
        if result.timed_out || result.rc != 0 {
            for entry in win32::list_processes() {
                if !entry.name.eq_ignore_ascii_case("adb.exe") {
                    continue;
                }
                if let Some(exe) = win32::proc_exe(entry.pid) {
                    if util::same_path(&exe, &self.adb_exe) {
                        util::taskkill_tree(entry.pid);
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    fn wait_device(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.adb_state() == "device" {
                return true;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        false
    }

    fn wait_boot(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let result = self.adb_shell(&["getprop", "sys.boot_completed"], Duration::from_secs(15));
            if result.stdout.trim() == "1" {
                return true;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        false
    }

    pub fn wait_adb_ready(&mut self) -> Result<(), String> {
        if let Ok(_) = self.refresh() {}
        log::info!("[ADB] 等待连接恢复: {}", self.serial);
        if self.settings.kill_adb_server {
            log::info!("[ADB] 结束旧 adb server 以清除卡死连接");
            self.kill_adb_server();
        }
        self.adb(&["start-server"], Duration::from_secs(60));
        let deadline = Instant::now() + Duration::from_secs(self.settings.adb_ready_timeout);
        let mut attempt = 0;
        while Instant::now() < deadline {
            attempt += 1;
            self.adb(&["connect", &self.serial], Duration::from_secs(30));
            if self.wait_device(Duration::from_secs(30)) && self.wait_boot(Duration::from_secs(120)) {
                let result = self.adb_shell(&["echo", "watchdog-ok"], Duration::from_secs(15));
                if result.stdout.contains("watchdog-ok") {
                    log::info!("[ADB] 连接已恢复（尝试 {attempt} 次）");
                    return Ok(());
                }
            }
            self.adb(&["disconnect", &self.serial], Duration::from_secs(15));
            std::thread::sleep(Duration::from_secs(3));
        }
        log::warn!("[ADB] 常规连接失败，尝试通过 MuMuManager 连接");
        let connect_result = self.raw(
            &["adb", "-v", &self.instance.to_string(), "-c", "connect"],
            Duration::from_secs(60),
        );
        match connect_result {
            Ok(value) => log::info!("[ADB] MuMuManager connect 返回: {value}"),
            Err(err) => log::warn!("[ADB] MuMuManager connect 失败: {err}"),
        }
        if self.wait_device(Duration::from_secs(30)) && self.wait_boot(Duration::from_secs(60)) {
            return Ok(());
        }
        Err(format!(
            "ADB 连接恢复超时（{} 秒），当前设备 {}",
            self.settings.adb_ready_timeout, self.serial
        ))
    }

    fn wait_manager_ready(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.info(None).is_ok() {
                log::info!("[MuMu] 控制接口已恢复");
                return true;
            }
            std::thread::sleep(Duration::from_secs(3));
        }
        false
    }

    pub fn ensure_manager(&self) -> Result<(), String> {
        if self.info(None).is_ok() {
            return Ok(());
        }
        log::warn!("[MuMu] 控制接口无响应，尝试启动/恢复 MuMu 主程序");
        let _ = self.raw(&["main", "launch"], Duration::from_secs(60));
        if self.wait_manager_ready(Duration::from_secs(45)) {
            return Ok(());
        }
        log::warn!("[MuMu] 主程序可能卡死，重启 MuMu 主程序（会关闭当前所有 MuMu 实例）");
        for entry in win32::list_processes() {
            if entry.name.eq_ignore_ascii_case("MuMuNxMain.exe") {
                util::taskkill_tree(entry.pid);
            }
        }
        std::thread::sleep(Duration::from_secs(2));
        let main_exe = self.manager.parent().map(|parent| parent.join("MuMuNxMain.exe"));
        let Some(main_exe) = main_exe else {
            return Err("找不到 MuMu 主程序目录".to_string());
        };
        if !main_exe.exists() {
            return Err(format!("找不到 MuMu 主程序 {}", main_exe.display()));
        }
        if let Some(parent) = main_exe.parent() {
            let _ = util::spawn_process(&main_exe, &[], Some(parent), false, true, false);
        }
        if self.wait_manager_ready(Duration::from_secs(90)) {
            Ok(())
        } else {
            Err("MuMu 控制接口无法恢复，请手动重启 MuMu 模拟器".to_string())
        }
    }

    fn wait_stopped(&self, timeout: Duration, fallback_pid: Option<u32>) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            match self.info(None) {
                Ok(info) => {
                    if !info
                        .get("is_process_started")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false)
                    {
                        return true;
                    }
                }
                Err(_) => {
                    if let Some(pid) = fallback_pid {
                        if !win32::is_pid_alive(pid) {
                            return true;
                        }
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        false
    }

    pub fn shutdown(&mut self) -> Result<(), String> {
        let info = match self.info(None) {
            Ok(info) => info,
            Err(_) => {
                self.ensure_manager()?;
                self.info(None)?
            }
        };
        if !info
            .get("is_process_started")
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
        {
            log::info!("[MuMu] 实例已是关闭状态");
            return Ok(());
        }
        let pid = info
            .get("pid")
            .and_then(|value| value.as_u64())
            .map(|pid| pid as u32);
        self.last_pid = pid;
        log::info!(
            "[MuMu] 关闭实例 {}（{}）...",
            self.instance,
            info.get("name").and_then(|value| value.as_str()).unwrap_or("")
        );
        let _ = self.raw(
            &["control", "-v", &self.instance.to_string(), "shutdown"],
            Duration::from_secs(120),
        );
        if self.wait_stopped(Duration::from_secs(self.settings.emulator_shutdown_timeout), pid) {
            log::info!("[MuMu] 实例已关闭");
            std::thread::sleep(Duration::from_secs(3));
            return Ok(());
        }
        log::warn!("[MuMu] 未能正常关闭，尝试强制结束实例进程");
        if let Some(pid) = pid {
            util::taskkill_tree(pid);
        }
        if self.wait_stopped(Duration::from_secs(30), pid) {
            log::info!("[MuMu] 实例已被强制关闭");
            std::thread::sleep(Duration::from_secs(2));
            return Ok(());
        }
        log::warn!("[MuMu] 仍然无法关闭，最后尝试重启 MuMu 主程序");
        for entry in win32::list_processes() {
            if entry.name.eq_ignore_ascii_case("MuMuNxMain.exe")
                || entry.name.eq_ignore_ascii_case("MuMuNxDevice.exe")
            {
                util::taskkill_tree(entry.pid);
            }
        }
        std::thread::sleep(Duration::from_secs(3));
        Ok(())
    }

    pub fn wait_started(&self, timeout: Duration) -> Result<Json, String> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Ok(info) = self.info(None) {
                if Self::is_running(&info) {
                    let state = info
                        .get("player_state")
                        .and_then(|value| value.as_str())
                        .unwrap_or("");
                    if state.is_empty() || state == "start_finished" {
                        return Ok(info);
                    }
                }
                if let Some(code) = info.get("launch_err_code").and_then(|value| value.as_i64()) {
                    if code != 0 {
                        log::warn!(
                            "[MuMu] 启动报错 {code}: {}",
                            info.get("launch_err_msg")
                                .and_then(|value| value.as_str())
                                .unwrap_or("")
                        );
                    }
                }
            }
            std::thread::sleep(Duration::from_secs(3));
        }
        Err(format!("等待 MuMu 实例启动超时（{} 秒）", timeout.as_secs()))
    }

    pub fn launch(&mut self) -> Result<Json, String> {
        self.ensure_manager()?;
        for attempt in 1..=3 {
            if let Ok(info) = self.info(None) {
                if Self::is_running(&info) {
                    log::info!("[MuMu] 实例已在运行");
                    let _ = self.refresh();
                    return Ok(info);
                }
            }
            log::info!("[MuMu] 启动实例 {}（第 {attempt} 次）...", self.instance);
            let _ = self.raw(
                &["control", "-v", &self.instance.to_string(), "launch"],
                Duration::from_secs(120),
            );
            match self.wait_started(Duration::from_secs(self.settings.emulator_boot_timeout)) {
                Ok(info) => {
                    let _ = self.refresh();
                    log::info!(
                        "[MuMu] 实例启动完成，pid={}，adb={}",
                        info.get("pid")
                            .and_then(|value| value.as_u64())
                            .map(|pid| pid.to_string())
                            .unwrap_or_else(|| "?".to_string()),
                        self.serial
                    );
                    std::thread::sleep(Duration::from_secs(5));
                    return Ok(info);
                }
                Err(err) => {
                    log::warn!("[MuMu] {err}");
                    let _ = self.raw(
                        &["control", "-v", &self.instance.to_string(), "shutdown"],
                        Duration::from_secs(60),
                    );
                    std::thread::sleep(Duration::from_secs(5));
                }
            }
        }
        Err("MuMu 实例多次启动失败".to_string())
    }

    pub fn emulator_start_time(&self) -> Option<chrono::DateTime<chrono::Local>> {
        self.last_pid.and_then(win32::proc_create_time)
    }

    pub fn memory_usage(&self) -> Option<(f64, f64)> {
        let pid = self.last_pid?;
        let (working_set, private) = win32::process_memory(pid)?;
        Some((
            working_set as f64 / 1024.0 / 1024.0 / 1024.0,
            private as f64 / 1024.0 / 1024.0 / 1024.0,
        ))
    }
}

pub fn _type_hint(_: &Path) {}
