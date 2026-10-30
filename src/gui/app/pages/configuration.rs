//! 页面内容布局，文字和颜色继承统一主题。
use super::super::*;
impl GuiApp {
    pub(in crate::gui::app) fn show_configuration(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &egui::Context,
    ) {
        if let Some(settings) = self.editor.show(ui, self.reload_pending.is_some()) {
            // 丢弃旧接收端后，原状态轮询线程会自行退出。
            let replacement = GuiApp::new(settings);
            self.settings = replacement.settings;
            self.receiver = replacement.receiver;
            self.status = Json::Null;
            self.request_reload();
        }
        if std::mem::take(&mut self.editor.reload_requested) {
            self.request_reload();
        }
    }
}
