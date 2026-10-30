//! 原生窗口入口；状态、页面和主题由各自模块负责。
use crate::{config::Settings, constants};
use eframe::egui;
use std::time::Duration;
mod app;
mod background;
mod fonts;
mod theme;
mod worker;
pub fn run(settings: Settings) -> Result<(), String> {
    let Some(_instance) = crate::instance::InstanceGuard::acquire("gui")? else {
        // 首个进程可能仍在创建原生窗口，稍后再尝试唤起。
        for _ in 0..50 {
            if crate::instance::restore_gui() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        return Ok(());
    };
    if crate::instance::restore_gui() {
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/watchdog.png"))
                    .map_err(|e| format!("加载窗口图标失败：{e}"))?,
            )
            .with_inner_size([1020.0, 700.0])
            .with_min_inner_size([680.0, 560.0])
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
            fonts::install(&cc.egui_ctx);
            Ok(Box::new(app::GuiApp::create(settings, &cc.egui_ctx)?))
        }),
    )
    .map_err(|err| format!("GUI 启动失败: {err}"))
}
