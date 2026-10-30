//! 页面内容布局，文字和颜色继承统一主题。
use super::super::*;
impl GuiApp {
    pub(in crate::gui::app) fn show_logs(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        ui.horizontal(|ui| {
            ui.heading("运行日志");
            if ui.button("打开日志目录").clicked() {
                let _ = std::process::Command::new("explorer")
                    .arg(&self.settings.log_dir)
                    .spawn();
            }
        });
        ui.label("最近 200 行 · 自动刷新");
        ui.add_space(12.0);
        egui::ScrollArea::both()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(&self.logs).text_style(egui::TextStyle::Monospace),
                    )
                    .selectable(true)
                    .wrap_mode(egui::TextWrapMode::Extend),
                );
            });
    }
}
