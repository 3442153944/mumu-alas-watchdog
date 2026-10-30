//! 页面内容布局，文字和颜色继承统一主题。
use super::super::*;
impl GuiApp {
    pub(in crate::gui::app) fn show_overview(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("运行概览");
            ui.add_space(14.0);
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
            });
            ui.add_space(14.0);
            for (title, key) in [
                ("看门狗", "watchdog"),
                ("看门狗资源", "self"),
                ("下次计划重启", "next_restart"),
                ("MuMu 模拟器", "emulator"),
                ("ADB 连接", "adb"),
                ("Alas 任务", "alas"),
            ] {
                egui::Frame::new()
                    .fill(egui::Color32::TRANSPARENT)
                    .corner_radius(10.0)
                    .inner_margin(12.0)
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.small(title);
                        ui.add_space(5.0);
                        ui.label(self.status_line(key));
                    });
                ui.add_space(6.0);
            }
            ui.add_space(8.0);
            ui.add_space(16.0);
            ui.collapsing("开机启动与监督任务", |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (label, action) in [
                        (
                            "安装开机自启",
                            crate::integrations::install_autostart as fn() -> Result<(), String>,
                        ),
                        ("移除开机自启", crate::integrations::uninstall_autostart),
                        ("安装监督者任务", crate::integrations::install_supervisor),
                        ("移除监督者任务", crate::integrations::uninstall_supervisor),
                    ] {
                        if ui.button(label).clicked() {
                            self.message = action()
                                .map(|()| format!("已完成：{label}"))
                                .unwrap_or_else(|e| e);
                        }
                    }
                });
            });
        });
    }
}
