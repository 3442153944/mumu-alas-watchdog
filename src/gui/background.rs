//! 背景图片解码与等比裁切，不改变原图内容。
use eframe::egui;
use std::path::Path;
pub(super) fn load_background(
    ctx: &egui::Context,
    path: Option<&Path>,
) -> Result<egui::TextureHandle, String> {
    let pixels = if let Some(path) = path {
        if std::fs::metadata(path)
            .map_err(|e| format!("无法读取背景图片：{e}"))?
            .len()
            > 32 * 1024 * 1024
        {
            return Err("背景图片不能超过 32 MB".into());
        }
        let mut reader = image::ImageReader::open(path)
            .map_err(|e| e.to_string())?
            .with_guessed_format()
            .map_err(|e| e.to_string())?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(8192);
        limits.max_image_height = Some(8192);
        limits.max_alloc = Some(256 * 1024 * 1024);
        reader.limits(limits);
        reader
            .decode()
            .map_err(|e| format!("背景图片无法解码：{e}"))?
            .into_rgba8()
    } else {
        image::load_from_memory(include_bytes!("../../assets/background.png"))
            .map_err(|e| e.to_string())?
            .into_rgba8()
    };
    Ok(ctx.load_texture(
        "user-background",
        egui::ColorImage::from_rgba_unmultiplied(
            [pixels.width() as usize, pixels.height() as usize],
            pixels.as_raw(),
        ),
        egui::TextureOptions::LINEAR,
    ))
}
pub(super) fn uv(view: egui::Vec2, image: egui::Vec2) -> egui::Rect {
    let view_aspect = view.x.max(1.0) / view.y.max(1.0);
    let image_aspect = image.x / image.y;
    if view_aspect < image_aspect {
        let width = view_aspect / image_aspect;
        let left = (1.0 - width) * 0.8;
        egui::Rect::from_min_max(egui::pos2(left, 0.0), egui::pos2(left + width, 1.0))
    } else {
        let height = image_aspect / view_aspect;
        let top = (1.0 - height) * 0.35;
        egui::Rect::from_min_max(egui::pos2(0.0, top), egui::pos2(1.0, top + height))
    }
}
#[cfg(test)]
mod tests {

    use super::*;
    #[test]
    fn cover_keeps_correct_aspect_at_common_window_sizes() {
        let image = egui::vec2(1672.0, 941.0);
        for view in [
            egui::vec2(680.0, 560.0),
            egui::vec2(1020.0, 700.0),
            egui::vec2(1920.0, 1080.0),
            egui::vec2(2560.0, 1080.0),
        ] {
            let uv = uv(view, image);
            assert!(
                uv.min.x >= 0.0 && uv.min.y >= 0.0 && uv.max.x <= 1.00001 && uv.max.y <= 1.00001
            );
            let cropped = image * uv.size();
            assert!((cropped.x / cropped.y - view.x / view.y).abs() < 0.0001);
        }
    }

    #[test]
    fn background_loader_supports_formats_and_rejects_invalid_data() {
        let ctx = egui::Context::default();
        let root = std::env::temp_dir().join(format!("muw-bg-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        for suffix in ["png", "jpg", "webp"] {
            let path = root.join(format!("background.{suffix}"));
            let pixels = image::RgbImage::from_pixel(4, 4, image::Rgb([80, 150, 190]));
            pixels.save(&path).unwrap();
            let texture = load_background(&ctx, Some(&path)).unwrap();
            assert_eq!(texture.size(), [4, 4]);
        }
        let bad = root.join("bad.png");
        std::fs::write(&bad, "not an image").unwrap();
        assert!(load_background(&ctx, Some(&bad)).is_err());
        assert!(load_background(&ctx, Some(&root.join("missing.png"))).is_err());
        for entry in std::fs::read_dir(&root).unwrap() {
            std::fs::remove_file(entry.unwrap().path()).unwrap();
        }
        std::fs::remove_dir(root).unwrap();
    }
}
