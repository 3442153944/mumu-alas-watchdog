//! egui 控制台：状态展示、启停/重启按钮、日志轮询、操作输出。

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use eframe::egui;
use serde_json::Value as Json;

use crate::config::Settings;
use crate::constants;
use crate::util;
use crate::watchdog::Watchdog;

enum Update {
    Status(Json),
    Error(String),
}

pub fn run(settings: Settings) -> Result<(), String> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1020.0, 700.0])
            .with_min_inner_size([860.0, 620.0])
            .with_title(format!(
                "{} v{} · MuMu + Alas 自动重启控制台",
                constants::APP_NAME,
                constants::VERSION
            )),
        ..Default::default()
    };
    eframe::run_native(
        "MuMuAlasWatchdog",
        options,
        Box::new(move |cc| {
            install_cjk_font(&cc.egui_ctx);
            Ok(Box::new(GuiApp::new(settings)))
        }),
    )
    .map_err(|err| format!("GUI 启动失败: {err}"))
}

fn install_cjk_font(ctx: &egui::Context) {
    const CANDIDATES: [&str; 4] = [
        r"C:\Windows\Fonts\simhei.ttf",
        r"C:\Windows\Fonts\simkai.ttf",
        r"C:\Windows\Fonts\simfang.ttf",
        r"C:\Windows\Fonts\msyh.ttc",
    ];
    for path in CANDIDATES {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "cjk".to_owned(),
            egui::FontData::from_owned(bytes).into(),
        );
        for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
            fonts
                .families
                .entry(family)
                .or_default()
                .insert(0, "cjk".to_owned());
        }
        ctx.set_fonts(fonts);
        return;
    }
}

struct GuiApp {
    settings: Settings,
    receiver: Receiver<Update>,
    status: Json,
    logs: String,
    message: String,
    last_log_refresh: Instant,
}

impl GuiApp {
    fn new(settings: Settings) -> Self {
        let (sender, receiver) = mpsc::channel();
        let worker_settings = settings.clone();
        std::thread::spawn(move || {
            let mut watchdog = match Watchdog::new(worker_settings) {
                Ok(watchdog) => watchdog,
                Err(err) => {
                    let _ = sender.send(Update::Error(err));
                    return;
                }
            };
            loop {
                let texts = watchdog.logwatch.scan();
                watchdog.logwatch.analyze(&texts);
                watchdog.refresh_emulator_runtime();
                let running = util::watcher_running_pid();
                let mut payload = if running.is_some() {
                    util::read_json(&constants::status_json())
                        .unwrap_or_else(|| watchdog.collect_status())
                } else {
                    watchdog.collect_status()
                };
                let missing_plan = payload
                    .pointer("/watchdog/next_restart")
                    .map(|value| value.is_null())
                    .unwrap_or(true);
                if missing_plan {
                    let estimate = watchdog.compute_next_restart(chrono::Local::now());
                    payload["watchdog"]["next_restart"] = serde_json::json!(format!(
                        "{}（估算）",
                        estimate.format("%Y-%m-%d %H:%M:%S")
                    ));
                }
                if sender.send(Update::Status(payload)).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        });
        GuiApp {
            settings,
            receiver,
            status: Json::Null,
            logs: String::new(),
            message: "就绪：看门狗未运行时可查看状态；点“启动看门狗”后后台常驻".to_string(),
            last_log_refresh: Instant::now() - Duration::from_secs(10),
        }
    }

    fn spawn_self(&mut self, args: &[&str], description: &str) {
        let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        match util::spawn_process(
            constants::app_path(),
            &args,
            Some(constants::wd_dir()),
            false,
            true,
            true,
        ) {
            Ok(pid) => self.message = format!("已触发：{description}（PID {pid}）"),
            Err(err) => self.message = format!("触发失败：{err}"),
        }
    }

    fn refresh_logs(&mut self) {
        if self.last_log_refresh.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.last_log_refresh = Instant::now();
        if let Some(path) = util::newest_log_file(&self.settings.log_dir) {
            self.logs = util::tail_lines(&path, 200, 512 * 1024).join("\n");
        }
    }

    fn drain_updates(&mut self) {
        while let Ok(update) = self.receiver.try_recv() {
            match update {
                Update::Status(payload) => self.status = payload,
                Update::Error(err) => self.message = err,
            }
        }
    }

    fn status_text(&self, pointer: &str, fallback: &str) -> String {
        self.status
            .pointer(pointer)
            .map(|value| match value {
                Json::String(text) => text.clone(),
                Json::Null => fallback.to_string(),
                other => other.to_string(),
            })
            .unwrap_or_else(|| fallback.to_string())
    }

    fn status_line(&self, key: &str) -> String {
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
                "WS {} MB / Handles {}",
                self.status_text("/watchdog/self_working_set_mb", "?"),
                self.status_text("/watchdog/self_handles", "?")
            ),
            _ => String::new(),
        }
    }
}

impl eframe::App for GuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.drain_updates();
        self.refresh_logs();
        ctx.request_repaint_after(Duration::from_millis(500));

        egui::TopBottomPanel::top("status").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.heading("运行状态（每 2 秒自动刷新）");
            egui::Grid::new("status_grid")
                .num_columns(2)
                .spacing([8.0, 4.0])
                .show(ui, |ui| {
                    for (label, key) in [
                        ("看门狗", "watchdog"),
                        ("下次重启", "next_restart"),
                        ("模拟器", "emulator"),
                        ("ADB", "adb"),
                        ("Alas", "alas"),
                        ("GUI 自身", "self"),
                    ] {
                        ui.label(format!("{label}："));
                        ui.label(self.status_line(key));
                        ui.end_row();
                    }
                });
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                if ui.button("启动看门狗").clicked() {
                    self.spawn_self(&["--start"], "启动看门狗");
                }
                if ui.button("停止看门狗").clicked() {
                    self.spawn_self(&["--stop"], "停止看门狗");
                }
                if ui.button("立即完整重启").clicked() {
                    self.spawn_self(&["--restart-now"], "完整重启（模拟器 + Alas）");
                }
                if ui.button("只重启 Alas").clicked() {
                    self.spawn_self(&["--restart-alas-now"], "只重启 Alas");
                }
                if ui.button("打开日志目录").clicked() {
                    let _ = std::process::Command::new("explorer")
                        .arg(self.settings.log_dir.as_os_str())
                        .spawn();
                }
                if ui.button("打开配置文件").clicked() {
                    let _ = std::process::Command::new("notepad")
                        .arg(self.settings.path.as_os_str())
                        .spawn();
                }
                if ui.button("安装开机自启").clicked() {
                    match crate::integrations::install_autostart() {
                        Ok(()) => self.message = "已安装开机自启".to_string(),
                        Err(err) => self.message = err,
                    }
                }
                if ui.button("移除开机自启").clicked() {
                    match crate::integrations::uninstall_autostart() {
                        Ok(()) => self.message = "已移除开机自启".to_string(),
                        Err(err) => self.message = err,
                    }
                }
                if ui.button("安装监督者任务").clicked() {
                    match crate::integrations::install_supervisor() {
                        Ok(()) => self.message = "已安装监督者计划任务（每 5 分钟 --ensure）".to_string(),
                        Err(err) => self.message = err,
                    }
                }
                if ui.button("移除监督者任务").clicked() {
                    match crate::integrations::uninstall_supervisor() {
                        Ok(()) => self.message = "已移除监督者计划任务".to_string(),
                        Err(err) => self.message = err,
                    }
                }
            });
            ui.add_space(2.0);
        });

        egui::TopBottomPanel::bottom("message").show(ctx, |ui| {
            ui.label(&self.message);
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("看门狗日志（自动刷新）");
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.logs.as_str())
                            .font(egui::TextStyle::Monospace)
                            .desired_width(f32::INFINITY)
                            .desired_rows(28),
                    );
                });
        });
    }
}

fn _assert_types(_: PathBuf) {}
