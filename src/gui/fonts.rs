//! 中文字体加载。
use eframe::egui;
use std::{fs::File, sync::OnceLock};

// 只读映射保留到进程退出，使 egui 能借用字体数据而不是复制整个字体集合。
static SYSTEM_FONT: OnceLock<Option<memmap2::Mmap>> = OnceLock::new();

fn definitions() -> egui::FontDefinitions {
    let mapped = SYSTEM_FONT.get_or_init(|| {
        for path in [
            r"C:\Windows\Fonts\msyh.ttc",
            r"C:\Windows\Fonts\simhei.ttf",
            r"C:\Windows\Fonts\simkai.ttf",
            r"C:\Windows\Fonts\simfang.ttf",
        ] {
            let Ok(file) = File::open(path) else {
                continue;
            };
            // 字体文件只读，映射由静态对象持有，字体借用不会早于映射失效。
            if let Ok(mapping) = unsafe { memmap2::MmapOptions::new().map(&file) } {
                return Some(mapping);
            }
        }
        None
    });
    let mut fonts = egui::FontDefinitions::default();
    if let Some(mapping) = mapped {
        fonts
            .font_data
            .insert("cjk".into(), egui::FontData::from_static(mapping).into());
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "cjk".into());
        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .push("cjk".into());
    }
    fonts
}
pub(super) fn install(ctx: &egui::Context) {
    ctx.set_fonts(definitions());
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mapped_font_is_borrowed_and_parses_without_copying_the_collection() {
        let fonts = definitions();
        if let Some(data) = fonts.font_data.get("cjk") {
            assert!(matches!(data.font, std::borrow::Cow::Borrowed(_)));
            let ctx = egui::Context::default();
            ctx.set_fonts(fonts);
            let _ = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.label("中文字体验证");
                });
            });
        }
    }
}
