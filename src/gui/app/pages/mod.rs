//! 页面路由；各页面不维护独立配色。
use super::*;
mod appearance;
mod configuration;
mod logs;
mod overview;
impl GuiApp {
    pub(super) fn show_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        match self.page {
            1 => self.show_configuration(ui, ctx),
            2 => self.show_logs(ui, ctx),
            3 => {
                egui::ScrollArea::vertical().show(ui, |ui| self.appearance_page(ui, ctx));
            }
            _ => self.show_overview(ui, ctx),
        }
    }
}
