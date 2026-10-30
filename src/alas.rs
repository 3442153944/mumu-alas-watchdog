//! Alas 控制：进程识别/结束、Win32 启动、WebUI 就绪检测、reloadalas 自动开始脚本。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::config::Settings;
use crate::layout::Layout;
use crate::util;
use crate::win32;

pub struct Alas {
    pub settings: Settings,
    pub layout: Layout,
}

impl Alas {
    pub fn new(settings: Settings, layout: Layout) -> Self {
        Alas { settings, layout }
    }

    fn electron_processes(&self) -> Vec<(u32, u32, PathBuf)> {
        let electron_dir = self.layout.alas_root.join("toolkit").join("webapp");
        let mut result = Vec::new();
        for entry in win32::list_processes() {
            if !entry.name.eq_ignore_ascii_case("alas.exe") {
                continue;
            }
            let Some(exe) = win32::proc_exe(entry.pid) else {
                continue;
            };
            if util::path_is_under(&exe, &electron_dir) {
                result.push((entry.pid, entry.ppid, exe));
            }
        }
        result
    }

    pub fn electron_process_count(&self) -> usize {
        self.electron_processes().len()
    }

    pub fn electron_main_pid(&self) -> Option<u32> {
        let processes = self.electron_processes();
        let pids: std::collections::HashSet<u32> = processes.iter().map(|(pid, _, _)| *pid).collect();
        processes
            .iter()
            .find(|(_, ppid, _)| !pids.contains(ppid))
            .map(|(pid, _, _)| *pid)
            .or_else(|| processes.first().map(|(pid, _, _)| *pid))
    }

    fn alas_processes(&self) -> Vec<(u32, PathBuf)> {
        let electron_dir = self.layout.alas_root.join("toolkit").join("webapp");
        let mut result = Vec::new();
        for entry in win32::list_processes() {
            let lower = entry.name.to_ascii_lowercase();
            if lower == "alas.exe" {
                if let Some(exe) = win32::proc_exe(entry.pid) {
                    if util::path_is_under(&exe, &electron_dir)
                        || util::same_path(&exe, &self.layout.launcher_exe)
                    {
                        result.push((entry.pid, exe));
                    }
                }
            } else if lower == "python.exe" || lower == "pythonw.exe" {
                if let Some(exe) = win32::proc_exe(entry.pid) {
                    if util::same_path(&exe, &self.layout.toolkit_python) {
                        result.push((entry.pid, exe));
                    }
                }
            }
        }
        result
    }

    pub fn webui_status(&self) -> (bool, u128) {
        util::http_probe(self.layout.webui_port, Duration::from_secs(4))
    }

    pub fn webui_ok(&self) -> bool {
        self.webui_status().0
    }

    fn find_port_owner(&self, port: u16) -> Option<u32> {
        let result = util::run_cmd("netstat", &["-ano", "-p", "TCP"], Duration::from_secs(20));
        for line in result.stdout.lines() {
            if !line.contains(&format!(":{port}")) || !line.to_ascii_uppercase().contains("LISTENING")
            {
                continue;
            }
            if let Some(pid) = line.split_whitespace().last() {
                if let Ok(pid) = pid.parse::<u32>() {
                    return Some(pid);
                }
            }
        }
        None
    }

    fn ensure_webui_port_free(&self) {
        let port = self.layout.webui_port;
        if !util::port_listening(port, Duration::from_secs(1)) {
            return;
        }
        if let Some(pid) = self.find_port_owner(port) {
            log::warn!("[Alas] WebUI 端口 {port} 仍被 PID {pid} 占用，强制结束");
            util::taskkill_tree(pid);
        }
        for _ in 0..20 {
            if !util::port_listening(port, Duration::from_secs(1)) {
                return;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    pub fn stop(&self) {
        let processes = self.alas_processes();
        if processes.is_empty() {
            log::info!("[Alas] 当前未在运行");
            self.ensure_webui_port_free();
            return;
        }
        log::info!("[Alas] 结束 Alas 进程（{} 个）...", processes.len());
        let electron = self.electron_processes();
        let pids: std::collections::HashSet<u32> = electron.iter().map(|(pid, _, _)| *pid).collect();
        let mut killed = std::collections::HashSet::new();
        for (pid, ppid, _) in &electron {
            if !pids.contains(ppid) {
                util::taskkill_tree(*pid);
                killed.insert(*pid);
            }
        }
        std::thread::sleep(Duration::from_millis(1500));
        let deadline = Instant::now() + Duration::from_secs(self.settings.kill_alas_timeout);
        while Instant::now() < deadline {
            let remaining = self.alas_processes();
            if remaining.is_empty() {
                break;
            }
            for (pid, _) in remaining {
                if killed.contains(&pid) {
                    continue;
                }
                util::taskkill_tree(pid);
                killed.insert(pid);
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        self.ensure_webui_port_free();
        log::info!("[Alas] 已结束");
    }

    pub fn write_reloadalas(&self) {
        if !self.settings.use_reloadalas {
            return;
        }
        let path = self.layout.alas_root.join("config").join("reloadalas");
        let text = format!("{}\n", self.layout.configs.join("\n"));
        match std::fs::write(&path, text) {
            Ok(_) => log::info!("[Alas] 已写入 {}（启动后自动开始游戏脚本）", path.display()),
            Err(err) => log::warn!("[Alas] 写入 reloadalas 失败: {err}"),
        }
    }

    pub fn start(&self) -> Result<u32, String> {
        let exe = self.layout.launch_exe();
        if !exe.exists() {
            return Err(format!("找不到 Alas 启动文件: {}", exe.display()));
        }
        log::info!(
            "[Alas] 通过 Win32 CreateProcessW 启动 Alas（{}）: {}",
            self.layout.launch_mode,
            exe.display()
        );
        let pid = util::spawn_process(exe, &[], Some(&self.layout.alas_root), true, true, false)
            .map_err(|err| format!("启动 Alas 失败: {err}"))?;
        log::info!("[Alas] 已创建进程，PID={pid}");
        Ok(pid)
    }

    pub fn wait_webui_ready(&self, pid: u32) -> Result<(), String> {
        log::info!("[Alas] 等待 WebUI 启动...");
        let deadline = Instant::now() + Duration::from_secs(self.settings.alas_start_timeout);
        let mut main_seen = false;
        while Instant::now() < deadline {
            if self.electron_main_pid().is_some() {
                main_seen = true;
            }
            if main_seen && self.webui_ok() {
                log::info!("[Alas] WebUI 已就绪（端口 {}）", self.layout.webui_port);
                return Ok(());
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        if !main_seen && !win32::is_pid_alive(pid) {
            return Err("Alas 启动进程已退出，且未检测到 Electron 主进程".to_string());
        }
        Err(format!(
            "等待 Alas WebUI 启动超时（{} 秒）",
            self.settings.alas_start_timeout
        ))
    }

    pub fn bring_up(&self) -> Result<(), String> {
        self.write_reloadalas();
        let pid = self.start()?;
        self.wait_webui_ready(pid)
    }

    pub fn executable_dir(&self) -> &Path {
        &self.layout.alas_root
    }
}
