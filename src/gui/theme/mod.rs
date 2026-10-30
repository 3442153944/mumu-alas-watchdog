//! 全局配色、控件状态与文字规则的唯一入口。
use crate::appearance::{normalized, Appearance};
use eframe::egui;
#[cfg(test)]
mod tests;
mod text;
mod widgets;
#[cfg(test)]
pub(super) use text::DARK_TEXT;
pub(super) use widgets::{content_panel, transparency_slider};
pub(super) const PANEL_BLUE: egui::Color32 = egui::Color32::from_rgb(0, 150, 250);
pub(super) fn surface(appearance: &Appearance) -> egui::Color32 {
    with_transparency(PANEL_BLUE, appearance.controls_transparency)
}
pub(super) fn apply(ctx: &egui::Context, appearance: &Appearance) {
    let mut style = base_style(ctx);
    let surface = surface(appearance);
    // 文字颜色由语义底色和控件透明度统一决定，页面不自行指定颜色。
    style.visuals.override_text_color = None;
    style.visuals.widgets.noninteractive.fg_stroke.color =
        text::foreground(PANEL_BLUE, appearance.controls_transparency);
    style.visuals.extreme_bg_color = surface;
    style.visuals.faint_bg_color = surface;
    style.visuals.window_fill = surface;
    style.visuals.panel_fill = surface;
    style.visuals.error_fg_color = egui::Color32::from_rgb(255, 190, 190);
    style.visuals.selection.bg_fill = with_transparency(
        egui::Color32::from_rgb(0, 105, 195),
        appearance.controls_transparency,
    );
    style.visuals.selection.stroke.color = text::foreground(
        egui::Color32::from_rgb(0, 105, 195),
        appearance.controls_transparency,
    );
    style.visuals.widgets.noninteractive.bg_fill = surface;
    style.visuals.widgets.noninteractive.weak_bg_fill = surface;
    for (widget, color) in [
        (&mut style.visuals.widgets.inactive, PANEL_BLUE),
        (
            &mut style.visuals.widgets.hovered,
            egui::Color32::from_rgb(0, 112, 205),
        ),
        (
            &mut style.visuals.widgets.active,
            egui::Color32::from_rgb(0, 82, 164),
        ),
        (
            &mut style.visuals.widgets.open,
            egui::Color32::from_rgb(0, 112, 205),
        ),
    ] {
        widget.bg_fill = with_transparency(color, appearance.controls_transparency);
        widget.weak_bg_fill = widget.bg_fill;
        widget.fg_stroke.color = text::foreground(color, appearance.controls_transparency);
    }
    ctx.set_style(style);
}
pub(super) fn with_transparency(color: egui::Color32, transparency: f32) -> egui::Color32 {
    color.gamma_multiply_u8(alpha(transparency))
}
pub(super) fn alpha(transparency: f32) -> u8 {
    ((1.0 - normalized(transparency, 0.0) / 100.0) * 255.0).round() as u8
}
fn base_style(ctx: &egui::Context) -> egui::Style {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::light();
    style.visuals.override_text_color = None;
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(14.0, 9.0);
    style.spacing.interact_size.y = 36.0;
    for (kind, size) in [
        (egui::TextStyle::Body, 17.0),
        (egui::TextStyle::Button, 16.0),
        (egui::TextStyle::Small, 14.0),
        (egui::TextStyle::Heading, 23.0),
        (egui::TextStyle::Monospace, 13.0),
    ] {
        style.text_styles.insert(
            kind.clone(),
            egui::FontId::new(
                size,
                if kind == egui::TextStyle::Monospace {
                    egui::FontFamily::Monospace
                } else {
                    egui::FontFamily::Proportional
                },
            ),
        );
    }
    style
}
