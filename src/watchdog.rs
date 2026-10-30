//! 看门狗主编排：重启流程、健康巡检、定时调度、守护循环、自我监控。

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};
use serde_json::{json, Value as Json};

use crate::alas::Alas;
use crate::config::Settings;
use crate::constants;
use crate::emulator::Emulator;
use crate::layout::{detect_layout, Layout};
use crate::logger;
use crate::logwatch::LogWatcher;
use crate::outln;
use crate::schedule;
use crate::status;
use crate::util;

pub struct Watchdog {
    pub settings: Settings,
    pub layout: Layout,
    pub emulator: Emulator,
    pub alas: Alas,
    pub logwatch: LogWatcher,
    pub emulator_start_time: Option<DateTime<Local>>,
    pending_full_cycle: Option<String>,
    pending_alas_restart: Option<String>,
    next_restart: Option<DateTime<Local>>,
    retry_at: Option<DateTime<Local>>,
    next_health: Option<DateTime<Local>>,
    next_status_log: Option<DateTime<Local>>,
    next_alas_restart_at: Option<DateTime<Local>>,
    next_stuck_trigger_at: Option<DateTime<Local>>,
    failures: u32,
    adb_fail: u32,
    emulator_missing: u32,
    mumu_fail: u32,
    alas_missing: u32,
    pub stop_requested: bool,
    cycle_count: u32,
    last_cycle: Option<Json>,
    started_at: DateTime<Local>,
    supervisor_installed: bool,
    self_restart_warned: bool,
    mumu_info_at: Option<Instant>,
    cpu_sample: Option<(Instant, f64)>,
    cpu_pct_one_core: Option<f64>,
}

impl Watchdog {
    pub fn new(settings: Settings) -> Result<Watchdog, String> {
        let layout = detect_layout(&settings)?;
        let emulator = Emulator::new(&settings, &layout);
        let alas = Alas::new(settings.clone(), layout.clone());
        let logwatch = LogWatcher::new(&settings, &layout);
        let mut watchdog = Watchdog {
            settings,
            layout,
            emulator,
            alas,
            logwatch,
            emulator_start_time: None,
            pending_full_cycle: None,
            pending_alas_restart: None,
            next_restart: None,
            retry_at: None,
            next_health: None,
            next_status_log: None,
            next_alas_restart_at: None,
            next_stuck_trigger_at: None,
            failures: 0,
            adb_fail: 0,
            emulator_missing: 0,
            mumu_fail: 0,
            alas_missing: 0,
            stop_requested: false,
            cycle_count: 0,
            last_cycle: None,
            started_at: Local::now(),
            supervisor_installed: false,
            self_restart_warned: false,
            mumu_info_at: None,
            cpu_sample: crate::win32::self_cpu_seconds().map(|cpu| (Instant::now(), cpu)),
            cpu_pct_one_core: None,
        };
        watchdog.supervisor_installed = supervisor_installed();
        watchdog.refresh_emulator_runtime();
        watchdog.print_layout();
        Ok(watchdog)
    }

    pub fn refresh_emulator_runtime(&mut self) {
        if let Ok(info) = self.emulator.info(None) {
            if info
                .get("is_process_started")
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
            {
                self.remember_emulator(&info);
            }
        }
    }

    fn remember_emulator(&mut self, info: &Json) {
        self.mumu_info_at = Some(Instant::now());
        if let Some(pid) = info.get("pid").and_then(|value| value.as_u64()) {
            self.emulator.last_pid = Some(pid as u32);
            if let Some(created) = crate::win32::proc_create_time(pid as u32) {
                self.emulator_start_time = Some(created);
            }
        }
        if let Some(name) = info.get("name").and_then(|value| value.as_str()) {
            self.layout.instance_name = Some(name.to_string());
        }
    }

    /// MuMuManager 单次查询实测要 100~220ms CPU，不能每分钟都跑：
    /// 平时靠缓存的 PID 做轻量存活检查，只有进程丢失或到达状态采集周期才重新查询。
    fn mumu_info_due(&self) -> bool {
        let refresh_every = self.settings.status_log_seconds.max(60);
        let due = self
            .mumu_info_at
            .map(|at| at.elapsed() >= Duration::from_secs(refresh_every))
            .unwrap_or(true);
        if due {
            return true;
        }
        match self.emulator.last_pid {
            Some(pid) => !crate::win32::is_pid_alive(pid),
            None => true,
        }
    }

    fn update_cpu_sample(&mut self) {
        let Some((then, then_cpu)) = self.cpu_sample else {
            self.cpu_sample = crate::win32::self_cpu_seconds().map(|cpu| (Instant::now(), cpu));
            return;
        };
        let now = Instant::now();
        let elapsed = now.duration_since(then).as_secs_f64();
        if elapsed < 2.0 {
            return;
        }
        if let Some(cpu) = crate::win32::self_cpu_seconds() {
            let pct = ((cpu - then_cpu) / elapsed * 100.0 * 100.0).round() / 100.0;
            self.cpu_pct_one_core = Some(pct);
            self.cpu_sample = Some((now, cpu));
        }
    }

    pub fn compute_next_restart(&self, now: DateTime<Local>) -> DateTime<Local> {
        schedule::compute_next_restart(
            now,
            &self.settings.restart_times,
            self.settings.restart_interval_hours,
            self.emulator_start_time,
        )
    }

    // ------------------------------------------------------------------
    // 重启流程
    // ------------------------------------------------------------------
    pub fn run_full_cycle(&mut self, reason: &str) -> bool {
        log::info!("{}", "=".repeat(70));
        log::info!("[重启] 开始完整重启（原因：{reason}）");
        let started = Instant::now();
        let snapshot = self.logwatch.offsets_snapshot();
        let mut success = false;
        let result = (|| -> Result<(), String> {
            log::info!("[重启] 步骤 1/6：结束 Alas");
            self.alas.stop();
            log::info!("[重启] 步骤 2/6：关闭 MuMu 实例");
            self.emulator.shutdown()?;
            log::info!("[重启] 步骤 3/6：启动 MuMu 实例");
            let info = self.emulator.launch()?;
            self.remember_emulator(&info);
            log::info!("[重启] 步骤 4/6：等待 ADB 恢复");
            self.emulator.wait_adb_ready()?;
            log::info!("[重启] 步骤 5/6：启动 Alas");
            self.alas.bring_up()?;
            log::info!("[重启] 步骤 6/6：等待游戏脚本自动开始");
            let electron_alive = || self.alas.electron_main_pid().is_some();
            let webui_ok = || self.alas.webui_ok();
            let mut offsets = snapshot.clone();
            self.logwatch.wait_scheduler_started(
                &mut offsets,
                &electron_alive,
                &webui_ok,
                self.settings.scheduler_start_timeout,
            )?;
            self.logwatch.restore_offsets(offsets);
            Ok(())
        })();
        match result {
            Ok(()) => {
                success = true;
                self.failures = 0;
                self.cycle_count += 1;
                self.logwatch.note_scheduler_started();
                self.last_cycle = Some(json!({
                    "time": util::now_text(),
                    "ok": true,
                    "reason": reason,
                    "duration_seconds": started.elapsed().as_secs(),
                }));
                log::info!(
                    "[重启] 完整重启成功，用时 {}",
                    util::fmt_duration(started.elapsed().as_secs_f64())
                );
            }
            Err(err) => {
                log::error!("[重启] 失败: {err}");
                self.last_cycle = Some(json!({
                    "time": util::now_text(),
                    "ok": false,
                    "reason": reason,
                    "duration_seconds": started.elapsed().as_secs(),
                }));
            }
        }
        self.log_emulator_memory("重启后");
        self.log_status_snapshot();
        log::info!("{}", "=".repeat(70));
        success
    }

    pub fn restart_alas_only(&mut self, reason: &str) -> bool {
        log::info!("{}", "=".repeat(70));
        log::info!("[重启] 开始重启 Alas（原因：{reason}，不重启模拟器）");
        let started = Instant::now();
        let snapshot = self.logwatch.offsets_snapshot();
        let result = (|| -> Result<(), String> {
            self.alas.stop();
            self.alas.bring_up()?;
            let electron_alive = || self.alas.electron_main_pid().is_some();
            let webui_ok = || self.alas.webui_ok();
            let mut offsets = snapshot.clone();
            self.logwatch.wait_scheduler_started(
                &mut offsets,
                &electron_alive,
                &webui_ok,
                self.settings.scheduler_start_timeout,
            )?;
            self.logwatch.restore_offsets(offsets);
            Ok(())
        })();
        match result {
            Ok(()) => {
                self.failures = 0;
                self.logwatch.note_scheduler_started();
                log::info!(
                    "[重启] Alas 重启成功，用时 {}",
                    util::fmt_duration(started.elapsed().as_secs_f64())
                );
                self.log_status_snapshot();
                true
            }
            Err(err) => {
                log::error!("[重启] Alas 重启失败: {err}");
                log::info!("{}", "=".repeat(70));
                false
            }
        }
    }

    pub fn ensure_running_on_start(&mut self) -> Result<(), String> {
        log::info!("[启动] 检查初始运行环境...");
        let info = match self.emulator.info(None) {
            Ok(info) => info,
            Err(_) => {
                self.emulator.ensure_manager()?;
                self.emulator.info(None)?
            }
        };
        if !Emulator::is_running(&info) {
            log::info!("[启动] 模拟器未运行，正在启动");
            let info = self.emulator.launch()?;
            self.remember_emulator(&info);
        }
        if self.emulator.adb_state() != "device" {
            log::info!("[启动] ADB 状态异常，尝试恢复连接");
            self.emulator.wait_adb_ready()?;
        }
        if let Some(pid) = self.alas.electron_main_pid() {
            log::info!("[启动] Alas 已在运行（Electron PID {pid}）");
            if !self.alas.webui_ok() {
                log::warn!("[启动] WebUI 暂时不可访问，交由巡检继续处理");
            }
            return Ok(());
        }
        log::info!("[启动] Alas 未运行，正在启动");
        let snapshot = self.logwatch.offsets_snapshot();
        self.alas.stop();
        self.alas.bring_up()?;
        let electron_alive = || self.alas.electron_main_pid().is_some();
        let webui_ok = || self.alas.webui_ok();
        let mut offsets = snapshot.clone();
        self.logwatch.wait_scheduler_started(
            &mut offsets,
            &electron_alive,
            &webui_ok,
            self.settings.scheduler_start_timeout,
        )?;
        self.logwatch.restore_offsets(offsets);
        Ok(())
    }

    // ------------------------------------------------------------------
    // 巡检
    // ------------------------------------------------------------------
    fn health_check(&mut self) {
        let texts = self.logwatch.scan();
        self.logwatch.analyze(&texts);
        self.update_cpu_sample();

        if self.mumu_info_due() {
            match self.emulator.info(None) {
                Ok(info) => {
                    self.mumu_fail = 0;
                    if !Emulator::is_running(&info) {
                        self.emulator_missing += 1;
                        log::warn!(
                            "[巡检] 检测到 MuMu 实例未运行（{}/2）",
                            self.emulator_missing
                        );
                        if self.emulator_missing >= 2 && self.settings.restart_if_emulator_down {
                            self.pending_full_cycle = Some("模拟器未运行".to_string());
                        }
                        return;
                    }
                    self.emulator_missing = 0;
                    self.remember_emulator(&info);
                }
                Err(err) => {
                    self.mumu_fail += 1;
                    log::warn!(
                        "[巡检] MuMu 信息查询失败（{}/{}）: {err}",
                        self.mumu_fail,
                        self.settings.adb_fail_threshold
                    );
                    if self.mumu_fail >= self.settings.adb_fail_threshold
                        && self.settings.restart_if_emulator_down
                    {
                        self.pending_full_cycle = Some("MuMu 控制接口无响应".to_string());
                    }
                    return;
                }
            }
        } else if let Some(pid) = self.emulator.last_pid {
            // 轻量存活检查，避免每分钟启动 MuMuManager（单次约 0.1~0.2s CPU）。
            if !crate::win32::is_pid_alive(pid) {
                self.emulator_missing += 1;
                log::warn!("[巡检] MuMu 进程已退出（{}/2）", self.emulator_missing);
                if self.emulator_missing >= 2 && self.settings.restart_if_emulator_down {
                    self.pending_full_cycle = Some("模拟器进程已退出".to_string());
                }
                return;
            }
            self.emulator_missing = 0;
        } else {
            return;
        }

        if self.settings.mumu_memory_limit_gb > 0.0 {
            if let Some((_working_set, commit_gb)) = self.emulator.memory_usage() {
                if commit_gb > self.settings.mumu_memory_limit_gb {
                    log::warn!(
                        "[巡检] 模拟器提交内存 {:.2} GB 超过阈值 {:.2} GB，触发提前重启",
                        commit_gb,
                        self.settings.mumu_memory_limit_gb
                    );
                    self.pending_full_cycle = Some(format!("模拟器内存超限（{commit_gb:.2} GB）"));
                    return;
                }
            }
        }

        let (state, adb_ms) = self.emulator.timed_adb_state();
        if state != "device" {
            self.adb_fail += 1;
            log::warn!(
                "[巡检] ADB 连接异常（{}/{}），当前: {}（{} ms）",
                self.adb_fail,
                self.settings.adb_fail_threshold,
                if state.is_empty() {
                    "无响应"
                } else {
                    &state
                },
                adb_ms
            );
            self.emulator
                .adb(&["connect", &self.emulator.serial], Duration::from_secs(30));
            if self.adb_fail >= self.settings.adb_fail_threshold {
                self.pending_full_cycle = Some("ADB 连接丢失".to_string());
            }
            return;
        }
        self.adb_fail = 0;
        self.check_alas_alive();
        self.check_task_stuck();
    }

    fn check_alas_alive(&mut self) {
        let main = self.alas.electron_main_pid();
        let (webui_ok, webui_ms) = self.alas.webui_status();
        if main.is_some() && webui_ok {
            self.alas_missing = 0;
            return;
        }
        self.alas_missing += 1;
        log::warn!(
            "[巡检] Alas 未正常运行（第 {} 次：Electron={}，WebUI={}，响应 {} ms）",
            self.alas_missing,
            if main.is_some() { "在" } else { "无" },
            if webui_ok { "正常" } else { "异常" },
            webui_ms
        );
        if self.alas_missing < 2 {
            return;
        }
        if self.logwatch.manual_stopped {
            return;
        }
        if !self.settings.restart_alas_if_closed {
            return;
        }
        if let Some(next) = self.next_alas_restart_at {
            if Local::now() < next {
                return;
            }
        }
        self.pending_alas_restart = Some("Alas 进程/WebUI 丢失".to_string());
        self.next_alas_restart_at =
            Some(Local::now() + chrono::Duration::seconds(self.settings.alas_restart_cooldown));
    }

    fn check_task_stuck(&mut self) {
        if self.settings.task_stuck_minutes <= 0 {
            return;
        }
        let now = Local::now();
        let mut candidate: Option<(String, String, Option<DateTime<Local>>, i64)> = None;
        for (config, state) in self.logwatch.task_state.iter() {
            let Some(task) = state.current_task.clone() else {
                continue;
            };
            let Some(last) = state.last_activity.or(state.task_started) else {
                continue;
            };
            let idle_minutes = (now - last).num_minutes();
            if idle_minutes < self.settings.task_stuck_minutes {
                continue;
            }
            candidate = Some((config.clone(), task, state.task_started, idle_minutes));
            break;
        }
        let Some((config, task, task_started, idle_minutes)) = candidate else {
            return;
        };
        if self
            .logwatch
            .mark_stuck_once(&config, &task, task_started.as_ref())
        {
            log::warn!("[巡检] 任务 {task} 已 {idle_minutes} 分钟没有任何日志输出，疑似卡死");
        }
        if self.settings.restart_on_task_stuck {
            let allowed = self
                .next_stuck_trigger_at
                .map(|next| now >= next)
                .unwrap_or(true);
            if allowed {
                self.pending_full_cycle =
                    Some(format!("任务 {task} 疑似卡死（{idle_minutes} 分钟无日志）"));
                self.next_stuck_trigger_at =
                    Some(now + chrono::Duration::seconds(self.settings.task_stuck_cooldown));
            }
        }
    }

    // ------------------------------------------------------------------
    // 控制文件 / 主循环
    // ------------------------------------------------------------------
    pub fn reload_config(&mut self) -> Result<(), String> {
        let settings = crate::reload::read_settings(&self.settings.path)?;
        let layout = detect_layout(&settings)?;
        let same_target = self.layout.alas_root == layout.alas_root
            && self.layout.mumu_manager == layout.mumu_manager
            && self.layout.instance == layout.instance
            && self.layout.configs == layout.configs
            && self.layout.serial == layout.serial;
        // Prepare every dependent object before committing. No stop/launch calls.
        let mut emulator = Emulator::new(&settings, &layout);
        let alas = Alas::new(settings.clone(), layout.clone());
        let logwatch = if same_target {
            None
        } else {
            Some(LogWatcher::new(&settings, &layout))
        };
        logger::reconfigure(&settings.log_dir, &settings.log_level, settings.keep_days)?;
        if same_target {
            emulator.last_pid = self.emulator.last_pid;
        }
        self.settings = settings;
        self.layout = layout;
        self.emulator = emulator;
        self.alas = alas;
        if let Some(logwatch) = logwatch {
            self.logwatch = logwatch;
            self.emulator_start_time = None;
            self.adb_fail = 0;
            self.emulator_missing = 0;
            self.mumu_fail = 0;
            self.alas_missing = 0;
            self.pending_full_cycle = None;
            self.pending_alas_restart = None;
            self.retry_at = None;
            self.next_alas_restart_at = None;
            self.next_stuck_trigger_at = None;
            self.failures = 0;
        }
        self.refresh_emulator_runtime();
        let now = Local::now();
        self.next_restart = Some(
            schedule::apply_startup_grace(
                self.compute_next_restart(now),
                now,
                self.settings.startup_grace_minutes,
            )
            .0,
        );
        self.next_health =
            Some(now + chrono::Duration::seconds(self.settings.check_interval as i64));
        self.next_status_log =
            Some(now + chrono::Duration::seconds(self.settings.status_log_seconds as i64));
        log::info!("[配置] 热重载成功，计划已重算；未执行启动时重启");
        let payload = self.collect_status();
        status::write_status(&self.settings, &payload);
        Ok(())
    }

    fn handle_reload(&mut self) {
        let path = crate::reload::request_path();
        let Ok(text) = fs::read_to_string(&path) else {
            return;
        };
        let _ = fs::remove_file(path);
        let Ok(request) = serde_json::from_str::<crate::reload::Request>(&text) else {
            log::warn!("[配置] 无效的热重载请求");
            return;
        };
        let result = if request.pid != std::process::id() {
            Err("看门狗进程已变化，请重新请求热重载".into())
        } else if !same_config_path(&request.path, &self.settings.path) {
            Err("后台使用另一份配置，不能热重载此文件".into())
        } else {
            self.reload_config()
        };
        let ok = result.is_ok();
        let message = result
            .err()
            .unwrap_or_else(|| "后台热重载成功，配置已生效".into());
        if !ok {
            log::error!("[配置] 热重载失败，保留原配置：{message}");
        }
        let reply = crate::reload::Reply {
            id: request.id,
            pid: std::process::id(),
            ok,
            message,
        };
        if let Err(err) = crate::reload::write_json(&crate::reload::reply_path(), &reply) {
            log::error!("[配置] 写入重载结果失败：{err}");
        }
    }

    fn handle_flags(&mut self) {
        if constants::flag_stop().exists() {
            let _ = fs::remove_file(constants::flag_stop());
            log::info!("收到停止请求，准备退出（若正在重启会等本次重启结束）");
            self.stop_requested = true;
            return;
        }
        self.handle_reload();
        if constants::flag_full_restart().exists() {
            let _ = fs::remove_file(constants::flag_full_restart());
            self.pending_full_cycle = Some("手动请求完整重启".to_string());
        }
        if constants::flag_alas_restart().exists() {
            let _ = fs::remove_file(constants::flag_alas_restart());
            self.pending_alas_restart = Some("手动请求重启 Alas".to_string());
        }
    }

    fn emulator_quick_ok(&self) -> bool {
        match self.emulator.info(None) {
            Ok(info) => Emulator::is_running(&info) && self.emulator.adb_state() == "device",
            Err(_) => false,
        }
    }

    fn tick(&mut self) {
        let now = Local::now();
        self.handle_flags();
        if self.stop_requested {
            return;
        }

        if let Some(reason) = self.pending_full_cycle.take() {
            if self.run_full_cycle(&reason) {
                self.retry_at = None;
                self.next_restart = Some(self.compute_next_restart(Local::now()));
                if let Some(next) = self.next_restart {
                    log::info!("[计划] 下一次定时重启: {}", schedule::format_local(next));
                }
            } else {
                self.failures += 1;
                let multiplier = 1u64 << std::cmp::min(self.failures.saturating_sub(1), 4);
                let delay = (self.settings.retry_delay * multiplier).min(1800);
                self.retry_at = Some(Local::now() + chrono::Duration::seconds(delay as i64));
                log::warn!(
                    "[计划] 重启失败，将在 {delay} 秒后重试（连续失败 {} 次）",
                    self.failures
                );
            }
            return;
        }

        if let Some(reason) = self.pending_alas_restart.take() {
            if !self.emulator_quick_ok() {
                self.pending_full_cycle = Some("重启 Alas 前发现模拟器/ADB 异常".to_string());
                return;
            }
            if self.restart_alas_only(&reason) {
                self.next_alas_restart_at = Some(
                    Local::now() + chrono::Duration::seconds(self.settings.alas_restart_cooldown),
                );
            } else {
                self.retry_at = Some(
                    Local::now() + chrono::Duration::seconds(self.settings.retry_delay as i64),
                );
            }
            return;
        }

        if let Some(retry_at) = self.retry_at {
            if now >= retry_at {
                self.retry_at = None;
                self.pending_full_cycle = Some("失败重试".to_string());
                return;
            }
        }

        if let Some(next) = self.next_restart {
            if now >= next {
                self.next_restart = None;
                self.pending_full_cycle = Some("定时重启".to_string());
                return;
            }
        }

        if self.next_health.map(|next| now >= next).unwrap_or(true) {
            self.next_health =
                Some(now + chrono::Duration::seconds(self.settings.check_interval as i64));
            self.health_check();
        }

        if self.next_status_log.map(|next| now >= next).unwrap_or(true) {
            self.next_status_log =
                Some(now + chrono::Duration::seconds(self.settings.status_log_seconds as i64));
            self.log_status_snapshot();
        }
    }

    pub fn run(&mut self) {
        if self.settings.restart_on_start {
            log::info!("[计划] restart_on_start = true，启动后立即完整重启");
            self.pending_full_cycle = Some("启动时立即重启".to_string());
        } else {
            let now = Local::now();
            let (next, graced) = schedule::apply_startup_grace(
                self.compute_next_restart(now),
                now,
                self.settings.startup_grace_minutes,
            );
            self.next_restart = Some(next);
            if graced {
                log::info!(
                    "[计划] 模拟器已运行较久，计划重启时间已到；为安全起见等待 {} 分钟后再执行",
                    self.settings.startup_grace_minutes
                );
            }
            log::info!("[计划] 下一次定时重启: {}", schedule::format_local(next));
            if self.settings.ensure_running_on_start {
                if let Err(err) = self.ensure_running_on_start() {
                    log::error!("[启动] 初始环境准备失败: {err}");
                }
            }
        }
        log::info!("[运行] 看门狗已开始运行");
        self.next_status_log =
            Some(Local::now() + chrono::Duration::seconds(self.settings.status_log_seconds as i64));
        self.next_health =
            Some(Local::now() + chrono::Duration::seconds(self.settings.check_interval as i64));
        self.log_status_snapshot();
        while !self.stop_requested {
            self.tick();
            if self.stop_requested {
                break;
            }
            std::thread::sleep(Duration::from_millis(1500));
        }
        log::info!("[运行] 看门狗已退出");
    }

    // ------------------------------------------------------------------
    // 状态输出
    // ------------------------------------------------------------------
    pub fn collect_status(&self) -> Json {
        let mut payload = status::build_status(
            &self.settings,
            &self.layout,
            &self.emulator,
            &self.alas,
            &self.logwatch,
            self.next_restart,
            self.cycle_count,
            self.failures,
            &self.last_cycle,
            self.started_at,
        );
        if let Some(pct) = self.cpu_pct_one_core {
            payload["watchdog"]["self_cpu_pct_one_core"] = json!(pct);
        }
        payload
    }

    pub fn write_status_json(&self) {
        status::write_status(&self.settings, &self.collect_status());
    }

    pub fn log_status_snapshot(&mut self) {
        self.update_cpu_sample();
        let payload = self.collect_status();
        for line in status::format_lines(&payload, self.next_restart) {
            if line.contains("查询失败") {
                log::warn!("{line}");
            } else {
                log::info!("{line}");
            }
        }
        status::write_status(&self.settings, &payload);
        // 状态采集已经查询过 MuMu；同步缓存，避免巡检重复调用 MuMuManager。
        if payload.pointer("/emulator/error").is_none() {
            self.mumu_info_at = Some(Instant::now());
            if let Some(pid) = payload.pointer("/emulator/pid").and_then(|value| value.as_u64()) {
                self.emulator.last_pid = Some(pid as u32);
                if self.emulator_start_time.is_none() {
                    self.emulator_start_time = crate::win32::proc_create_time(pid as u32);
                }
            }
        }
        let tip = format!(
            "MuMu + Alas 看门狗（运行中） | 下次重启: {}",
            self.next_restart
                .map(|time| time.format("%Y-%m-%d %H:%M:%S").to_string())
                .unwrap_or_else(|| "尚未安排".to_string())
        );
        crate::tray::set_tip(&tip);
        self.check_self_restart(&payload);
    }

    fn check_self_restart(&mut self, payload: &Json) {
        let working_set = payload
            .pointer("/watchdog/self_working_set_mb")
            .and_then(|value| value.as_f64())
            .unwrap_or(0.0);
        let handles = payload
            .pointer("/watchdog/self_handles")
            .and_then(|value| value.as_u64())
            .unwrap_or(0) as u32;
        let uptime_hours = payload
            .pointer("/watchdog/uptime_seconds")
            .and_then(|value| value.as_i64())
            .unwrap_or(0) as f64
            / 3600.0;
        let mut reasons: Vec<String> = Vec::new();
        if self.settings.self_restart_memory_mb > 0
            && working_set > self.settings.self_restart_memory_mb as f64
        {
            reasons.push(format!(
                "内存 {working_set:.0} MB > {} MB",
                self.settings.self_restart_memory_mb
            ));
        }
        if self.settings.self_restart_handles > 0 && handles > self.settings.self_restart_handles {
            reasons.push(format!(
                "句柄 {handles} > {}",
                self.settings.self_restart_handles
            ));
        }
        if self.settings.self_restart_after_hours > 0
            && uptime_hours > self.settings.self_restart_after_hours as f64
        {
            reasons.push(format!(
                "运行 {uptime_hours:.1} 小时 > {} 小时",
                self.settings.self_restart_after_hours
            ));
        }
        if reasons.is_empty() {
            return;
        }
        let reason = reasons.join("；");
        if self.supervisor_installed {
            log::warn!("[自监控] 触发有序自重启：{reason}（由计划任务监督者拉起）");
            std::process::exit(0);
        }
        if !self.self_restart_warned {
            self.self_restart_warned = true;
            log::warn!(
                "[自监控] 资源超阈值（{reason}），但未安装监督者任务；为避免看门狗停摆，本次不退出。请执行 --install-supervisor"
            );
        }
    }

    fn log_emulator_memory(&self, tag: &str) {
        if let Some((working_set, commit)) = self.emulator.memory_usage() {
            log::info!("[{tag}] MuMu 内存: 工作集 {working_set:.2} GB, 提交 {commit:.2} GB");
        }
    }

    pub fn print_layout(&self) {
        log::info!("{}", "-".repeat(70));
        log::info!(
            "{} v{}（Rust/egui）",
            constants::APP_NAME,
            constants::VERSION
        );
        log::info!("[配置] {}", self.settings.path.display());
        log::info!("[路径] Alas: {}", self.layout.alas_root.display());
        log::info!("[路径] MuMuManager: {}", self.layout.mumu_manager.display());
        log::info!(
            "[路径] 实例 {}（{}） ADB: {}",
            self.layout.instance,
            self.layout.instance_name.as_deref().unwrap_or("未知"),
            self.emulator.serial
        );
        log::info!(
            "[路径] 配置: {} | WebUI 端口: {}",
            self.layout.configs.join(", "),
            self.layout.webui_port
        );
        log::info!(
            "[路径] 启动方式: {} -> {}",
            self.layout.launch_mode,
            self.layout.launch_exe().display()
        );
        log::info!("[路径] adb.exe: {}", self.layout.adb_exe.display());
        log::info!(
            "[日志] 目录: {}（保留 {} 天）",
            self.settings.log_dir.display(),
            self.settings.keep_days
        );
        if self.settings.restart_times.is_empty() {
            log::info!(
                "[计划] 每 {} 小时执行完整重启",
                self.settings.restart_interval_hours
            );
        } else {
            let times: Vec<String> = self
                .settings
                .restart_times
                .iter()
                .map(|(hour, minute)| format!("{hour:02}:{minute:02}"))
                .collect();
            log::info!("[计划] 每天 {} 执行完整重启", times.join(", "));
        }
        log::info!("{}", "-".repeat(70));
    }

    pub fn print_status(&self) {
        outln!("{}", "=".repeat(78));
        outln!(
            "MuMuAlasWatchdog(Rust)  状态检查    时间: {}",
            util::now_text()
        );
        outln!("{}", "=".repeat(78));
        outln!(
            "看门狗进程: {}",
            match util::watcher_running_pid() {
                Some(pid) => format!("运行中，PID {pid}"),
                None => "未运行".to_string(),
            }
        );
        outln!("配置文件:   {}", self.settings.path.display());
        outln!("日志目录:   {}", self.settings.log_dir.display());
        outln!("状态文件:   {}", constants::status_json().display());
        outln!("{}", "-".repeat(78));
        outln!("Alas 目录:      {}", self.layout.alas_root.display());
        outln!("MuMuManager:    {}", self.layout.mumu_manager.display());
        outln!(
            "MuMu 实例:      {}（{}）",
            self.layout.instance,
            self.layout.instance_name.as_deref().unwrap_or("未知")
        );
        outln!("Alas 配置:      {}", self.layout.configs.join(", "));
        outln!("WebUI 端口:     {}", self.layout.webui_port);
        outln!("ADB serial:     {}", self.emulator.serial);
        outln!("ADB 可执行文件: {}", self.layout.adb_exe.display());
        outln!(
            "Alas 启动方式:  {} -> {}",
            self.layout.launch_mode,
            self.layout.launch_exe().display()
        );
        outln!("{}", "-".repeat(78));

        match self.emulator.info(None) {
            Ok(info) => {
                outln!("[MuMu 模拟器]");
                for (key, label) in [
                    ("name", "名称"),
                    ("index", "实例编号"),
                    ("is_process_started", "进程已启动"),
                    ("is_android_started", "Android 已启动"),
                    ("player_state", "状态"),
                    ("pid", "进程 PID"),
                    ("adb_port", "ADB 端口"),
                ] {
                    if let Some(value) = info.get(key) {
                        let text = value
                            .as_str()
                            .map(|text| text.to_string())
                            .unwrap_or_else(|| value.to_string());
                        outln!("  {label:<14}: {text}");
                    }
                }
                if let Some(pid) = info.get("pid").and_then(|value| value.as_u64()) {
                    if let Some((working_set, commit)) = crate::win32::process_memory(pid as u32) {
                        outln!(
                            "  {:<14}: 工作集 {:.2} GB / 提交 {:.2} GB",
                            "内存",
                            working_set as f64 / 1024.0 / 1024.0 / 1024.0,
                            commit as f64 / 1024.0 / 1024.0 / 1024.0
                        );
                    }
                    if let Some(created) = crate::win32::proc_create_time(pid as u32) {
                        outln!(
                            "  {:<14}: {}（已运行 {}）",
                            "启动时间",
                            created.format("%Y-%m-%d %H:%M:%S"),
                            util::fmt_duration((Local::now() - created).num_seconds() as f64)
                        );
                    }
                }
            }
            Err(err) => outln!("[MuMu 模拟器] 查询失败: {err}"),
        }

        outln!("{}", "-".repeat(78));
        outln!("[ADB]");
        let devices = self.emulator.adb(&["devices"], Duration::from_secs(20));
        for line in devices.stdout.lines() {
            outln!("  {line}");
        }
        let (state, response_ms) = self.emulator.timed_adb_state();
        outln!(
            "  serial {} 状态: {}（响应 {} ms）",
            self.emulator.serial,
            if state.is_empty() {
                "无响应"
            } else {
                &state
            },
            response_ms
        );
        let boot = self
            .emulator
            .adb_shell(&["getprop", "sys.boot_completed"], Duration::from_secs(15));
        outln!("  sys.boot_completed: {}", boot.stdout.trim());

        outln!("{}", "-".repeat(78));
        outln!("[Alas]");
        let (webui_ok, webui_ms) = self.alas.webui_status();
        outln!(
            "  Electron 进程数: {}，主进程 PID {}",
            self.alas.electron_process_count(),
            self.alas
                .electron_main_pid()
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "无".to_string())
        );
        outln!(
            "  WebUI 访问: {}（响应 {} ms）",
            if webui_ok { "正常" } else { "异常" },
            webui_ms
        );
        let task_dict = self.logwatch.status_dict();
        match task_dict
            .get("current_task")
            .and_then(|value| value.as_str())
        {
            Some(task) => outln!(
                "  当前任务: {task}（开始于 {}，最近日志 {}）",
                task_dict
                    .get("task_started")
                    .and_then(|value| value.as_str())
                    .unwrap_or("?"),
                task_dict
                    .get("last_activity")
                    .and_then(|value| value.as_str())
                    .unwrap_or("?")
            ),
            None => outln!("  当前任务: 空闲/等待中"),
        }
        match self.logwatch.last_start_line() {
            Some((file, line)) => outln!("  最近调度器启动: {file}: {line}"),
            None => outln!("  最近调度器启动: 未在日志中找到"),
        }

        outln!("{}", "-".repeat(78));
        outln!("[计划]");
        if self.settings.restart_times.is_empty() {
            outln!(
                "  重启模式: 每 {} 小时",
                self.settings.restart_interval_hours
            );
        } else {
            let times: Vec<String> = self
                .settings
                .restart_times
                .iter()
                .map(|(hour, minute)| format!("{hour:02}:{minute:02}"))
                .collect();
            outln!("  重启模式: 每天 {}", times.join(", "));
        }
        let next = self.compute_next_restart(Local::now());
        if next >= Local::now() {
            outln!("  预计下次重启: {}", schedule::format_local(next));
        } else {
            outln!(
                "  计划重启时间: {}（已到期，启动后等待 {} 分钟宽限执行）",
                schedule::format_local(next),
                self.settings.startup_grace_minutes
            );
        }
        let (working_set, private_mb, handles) = logger::self_metrics();
        outln!(
            "  看门狗自身: WS {working_set:.1} MB / Private {private_mb:.1} MB / Handles {handles}"
        );
        outln!("{}", "=".repeat(78));
    }
}

// ---------------------------------------------------------------------------
// 单实例锁 / 守护进程
// ---------------------------------------------------------------------------

pub fn acquire_pid_lock() -> Result<Option<File>, String> {
    let path = constants::pid_file();
    for _ in 0..2 {
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                let _ = write!(file, "{}", std::process::id());
                let _ = file.flush();
                return Ok(Some(file));
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                let pid = fs::read_to_string(&path)
                    .ok()
                    .and_then(|text| text.trim().parse::<u32>().ok());
                if let Some(pid) = pid {
                    if crate::win32::is_pid_alive(pid) {
                        return Ok(None);
                    }
                }
                let _ = fs::remove_file(&path);
            }
            Err(err) => return Err(format!("创建 PID 文件失败: {err}")),
        }
    }
    Ok(None)
}

pub fn release_pid_lock(file: Option<File>) {
    drop(file);
    let _ = fs::remove_file(constants::pid_file());
}

pub fn supervisor_installed() -> bool {
    let result = util::run_cmd(
        "schtasks",
        &["/Query", "/TN", constants::SUPERVISOR_TASK_NAME],
        Duration::from_secs(20),
    );
    result.rc == 0
}

pub fn run_daemon(settings: Settings) -> Result<(), String> {
    let Some(_instance) = crate::instance::InstanceGuard::acquire("watchdog")? else {
        outln!("看门狗已在运行，本次不重复启动");
        return Ok(());
    };
    let Some(lock) = acquire_pid_lock()? else {
        outln!("MuMuAlasWatchdog 已经在运行，本次不重复启动");
        return Ok(());
    };
    for path in [
        constants::flag_stop(),
        constants::flag_full_restart(),
        constants::flag_alas_restart(),
        crate::reload::request_path(),
    ] {
        let _ = fs::remove_file(path);
    }
    let mut tray = match crate::tray::Tray::start("MuMu + Alas 看门狗（运行中）") {
        Ok(tray) => Some(tray),
        Err(err) => {
            log::warn!("托盘图标初始化失败（后台仍会运行）: {err}");
            None
        }
    };
    let result = (|| -> Result<(), String> {
        let mut watchdog = Watchdog::new(settings)?;
        watchdog.run();
        Ok(())
    })();
    if let Some(tray) = tray.as_mut() {
        tray.shutdown();
    }
    release_pid_lock(Some(lock));
    result
}

fn same_config_path(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod reload_tests {
    use super::*;
    #[test]
    fn reload_changes_live_settings_without_startup_actions_and_rolls_back_errors() {
        // Empty executable fixtures fail read-only queries immediately. They cannot
        // launch an emulator, ADB or Alas, and no real application paths are used.
        let root = std::env::temp_dir().join(format!("muw-live-reload-{}", std::process::id()));
        fs::create_dir_all(root.join("config")).unwrap();
        for name in ["alas.py", "gui.py", "Alas.exe", "manager.exe", "adb.exe"] {
            fs::write(root.join(name), "").unwrap();
        }
        fs::write(root.join("config/alas.json"), "{}").unwrap();
        fs::write(
            root.join("config/deploy.yaml"),
            "Deploy:\n  Webui:\n    WebuiPort: 9\n",
        )
        .unwrap();
        let path = root.join("config.yml");
        let mut raw: serde_yaml::Value =
            serde_yaml::from_str(crate::config::DEFAULT_CONFIG_TEXT).unwrap();
        raw["paths"]["alas_root"] = serde_yaml::to_value(&root).unwrap();
        raw["paths"]["mumu_manager"] = serde_yaml::to_value(root.join("manager.exe")).unwrap();
        raw["paths"]["adb_exe"] = serde_yaml::to_value(root.join("adb.exe")).unwrap();
        raw["paths"]["mumu_instance"] = serde_yaml::to_value(0).unwrap();
        raw["logging"]["dir"] = serde_yaml::to_value(root.join("logs")).unwrap();
        raw["monitor"]["write_status_json"] = serde_yaml::Value::Bool(false);
        fs::write(&path, serde_yaml::to_string(&raw).unwrap()).unwrap();
        let mut watchdog = Watchdog::new(Settings::load(&path).unwrap()).unwrap();
        watchdog.logwatch.manual_stopped = true;
        watchdog.adb_fail = 2;
        let started_at = watchdog.started_at;
        raw["schedule"]["restart_interval_hours"] = serde_yaml::to_value(12).unwrap();
        raw["schedule"]["restart_on_start"] = serde_yaml::Value::Bool(true);
        raw["health"]["check_interval_seconds"] = serde_yaml::to_value(120).unwrap();
        fs::write(&path, serde_yaml::to_string(&raw).unwrap()).unwrap();
        watchdog.reload_config().unwrap();
        assert_eq!(watchdog.settings.restart_interval_hours, 12.0);
        assert_eq!(watchdog.settings.check_interval, 120);
        assert_eq!(watchdog.emulator.instance, 0);
        assert!(watchdog.pending_full_cycle.is_none());
        assert!(watchdog.logwatch.manual_stopped);
        assert_eq!(watchdog.adb_fail, 2);
        assert_eq!(watchdog.started_at, started_at);
        let next = watchdog.next_restart;
        fs::write(&path, "health: {check_interval_seconds: broken}").unwrap();
        assert!(watchdog.reload_config().is_err());
        assert_eq!(watchdog.settings.check_interval, 120);
        assert_eq!(watchdog.next_restart, next);
        assert!(watchdog.logwatch.manual_stopped);
        drop(watchdog);
        fs::remove_dir_all(root).unwrap();
    }
}
