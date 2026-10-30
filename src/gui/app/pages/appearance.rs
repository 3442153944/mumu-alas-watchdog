//! 外观页面，只编排外观设置和背景选择。
use super::super::*;
impl GuiApp {
    fn choose_background(&mut self, ctx: &egui::Context, path: Option<std::path::PathBuf>) {
        match background::load_background(ctx, path.as_deref()) {
            Ok(texture) => {
                self.background = Some(texture);
                self.appearance.background_path = path;
                self.message = "背景已预览，点击“保存外观”可在下次启动时保留".into();
            }
            Err(error) => self.message = format!("{error}；保留当前背景"),
        }
    }
    pub(in crate::gui::app) fn appearance_page(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let before = self.appearance.clone();
        egui::Frame::new().inner_margin(18.0).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.heading("外观与背景");
            ui.label("拖动即时预览；淡蓝色底随透明度变化，滑轨和数值始终清晰。");
            ui.add_space(12.0);
            ui.label("背景透明度 · 0% 完整显示，100% 隐藏图片");
            theme::transparency_slider(ui, &mut self.appearance.background_transparency);
            ui.label("控件透明度 · 0% 不透明，100% 完全透明");
            theme::transparency_slider(ui, &mut self.appearance.controls_transparency);
            ui.add_space(12.0);
            ui.label(format!("当前背景：{}", self.appearance.background_path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "内置庭院插画".into())));
            ui.horizontal_wrapped(|ui| {
                if ui.button("选择背景图片…").clicked() {
                    if let Some(path) = rfd::FileDialog::new().set_directory(constants::wd_dir().join("assets")).add_filter("背景图片", &["png", "jpg", "jpeg", "webp"]).pick_file() { self.choose_background(ctx, Some(path)); }
                }
                if ui.button("使用内置背景").clicked() { self.choose_background(ctx, None); }
                if ui.button("恢复默认外观").clicked() {
                    self.choose_background(ctx, None);
                    self.appearance = Appearance::default();
                }
                if ui.add_enabled(self.appearance != self.saved_appearance, egui::Button::new("保存外观")).clicked() {
                    match self.appearance.save(&Appearance::path()) {
                        Ok(()) => { self.saved_appearance = self.appearance.clone(); self.message = "外观已保存，立即生效；下次启动自动恢复".into(); }
                        Err(error) => self.message = format!("保存外观失败：{error}"),
                    }
                }
            });
            ui.small("支持 PNG / JPEG / WebP；图片等比铺满，窗口变化时裁切。保存图片路径后，请保留原图片文件。");
            if self.appearance != self.saved_appearance { ui.label("当前为未保存的外观预览"); }
        });
        if before != self.appearance {
            theme::apply(ctx, &self.appearance);
        }
    }
}
