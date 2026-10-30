//! 根据控件自身的语义底色统一选择前景色，不用背景图片的平均色猜测文字颜色。
use eframe::egui::Color32;
pub(in crate::gui) const DARK_TEXT: Color32 = Color32::from_rgb(27, 46, 67);
/// 蓝色底较实的时候用白字；透明度达到 65% 后切换深字，避免透明区白字难辨。
/// 透明度来自控件自身，固定不透明的滑条仍按其实际底色计算。
pub(super) fn foreground(background: Color32, transparency: f32) -> Color32 {
    let linear = |channel: u8| {
        let value = channel as f32 / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    let brightness = 0.2126 * linear(background.r())
        + 0.7152 * linear(background.g())
        + 0.0722 * linear(background.b());
    if brightness < 0.38 && crate::appearance::normalized(transparency, 0.0) < 65.0 {
        Color32::WHITE
    } else {
        DARK_TEXT
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn foreground_follows_light_and_dark_control_surfaces() {
        for color in [
            Color32::from_rgb(0, 150, 250),
            Color32::from_rgb(0, 82, 164),
        ] {
            assert_eq!(foreground(color, 0.0), Color32::WHITE);
            assert_eq!(foreground(color, 55.0), Color32::WHITE);
            assert_eq!(foreground(color, 64.0), Color32::WHITE);
            assert_eq!(foreground(color, 65.0), DARK_TEXT);
            assert_eq!(foreground(color, 84.0), DARK_TEXT);
            assert_eq!(foreground(color, 100.0), DARK_TEXT);
        }
        assert_eq!(foreground(Color32::WHITE, 0.0), DARK_TEXT);
    }
}
