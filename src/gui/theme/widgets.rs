//! 共用底板、滑条和主要操作按钮；页面不重复配置颜色。
use super::*;
pub(in crate::gui) fn content_panel<R>(
    ctx: &egui::Context,
    appearance: &Appearance,
    contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    egui::CentralPanel::default()
        .frame(
            egui::Frame::new()
                .fill(surface(appearance))
                .inner_margin(24.0),
        )
        .show(ctx, |ui| {
            ui.set_min_width(ui.available_width());
            contents(ui)
        })
}
pub(in crate::gui) fn transparency_slider(ui: &mut egui::Ui, value: &mut f32) -> egui::Response {
    egui::Frame::new()
        .fill(with_transparency(PANEL_BLUE, 25.0))
        .stroke(egui::Stroke::new(
            1.0,
            egui::Color32::WHITE.gamma_multiply_u8(130),
        ))
        .corner_radius(8.0)
        .inner_margin(egui::vec2(12.0, 6.0))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let width = (ui.available_width() - 86.0).max(120.0);
            let style = ui.style_mut();
            style.spacing.slider_width = width;
            style.spacing.slider_rail_height = 8.0;
            style.visuals.extreme_bg_color = PANEL_BLUE;
            style.visuals.override_text_color = None;
            style.visuals.widgets.noninteractive.fg_stroke.color =
                text::foreground(PANEL_BLUE, 0.0);
            style.visuals.selection.bg_fill = PANEL_BLUE;
            style.visuals.selection.stroke.color = egui::Color32::WHITE;
            for widget in [
                &mut style.visuals.widgets.inactive,
                &mut style.visuals.widgets.hovered,
                &mut style.visuals.widgets.active,
            ] {
                widget.bg_fill = PANEL_BLUE;
                widget.weak_bg_fill = PANEL_BLUE;
                widget.fg_stroke = egui::Stroke::new(2.0, text::foreground(PANEL_BLUE, 0.0));
            }
            ui.add(
                egui::Slider::new(value, 0.0..=100.0)
                    .suffix("%")
                    .step_by(1.0)
                    .trailing_fill(true),
            )
        })
        .inner
}
