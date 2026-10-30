//! 四页真实绘制输出回归测试。

use super::*;

fn collect_text(shape: &egui::Shape, found: &mut Vec<(String, egui::Color32)>) {
    match shape {
        egui::Shape::Text(text) => {
            for section in &text.galley.job.sections {
                let color = text.override_text_color.unwrap_or_else(|| {
                    if section.format.color == egui::Color32::PLACEHOLDER {
                        text.fallback_color
                    } else {
                        section.format.color
                    }
                });
                found.push((
                    text.galley.job.text[section.byte_range.clone()].to_owned(),
                    color,
                ));
            }
        }
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_text(shape, found);
            }
        }
        _ => {}
    }
}

#[test]
fn all_four_pages_invert_normal_text_with_control_transparency() {
    // 构造无后台线程的界面，直接检查真实页面输出，不连接或控制实际应用。
    let path = std::env::temp_dir().join(format!("muw-page-text-{}.yml", std::process::id()));
    std::fs::write(&path, crate::config::DEFAULT_CONFIG_TEXT).unwrap();
    let settings = Settings::from_text(&path, crate::config::DEFAULT_CONFIG_TEXT, false).unwrap();
    let appearance = Appearance::default();
    let mut app = GuiApp {
        editor: ConfigEditor::load(&path),
        settings,
        page: 0,
        styled: true,
        receiver: worker::Updates::default(),
        status: serde_json::json!({}),
        logs: "日志正文测试 INFO 模拟器正常".into(),
        message: "就绪".into(),
        last_log_refresh: Instant::now(),
        reload_pending: None,
        background: None,
        appearance: appearance.clone(),
        saved_appearance: appearance,
    };
    let expected: [&[&str]; 4] = [
        &[
            "运行概览",
            "看门狗",
            "下次计划重启",
            "MuMu 模拟器",
            "ADB 连接",
            "Alas 任务",
        ],
        &[
            "配置",
            "保存后自动请求后台热重载",
            "Alas 根目录",
            "MuMuManager.exe",
            "MuMu 实例编号",
            "adb.exe",
            "Alas 启动方式",
        ],
        &["运行日志", "最近 200 行", "日志正文测试"],
        &[
            "外观与背景",
            "拖动即时预览",
            "背景透明度",
            "控件透明度",
            "当前背景",
            "支持 PNG",
        ],
    ];
    for (transparency, expected_color) in [
        (0.0, egui::Color32::WHITE),
        (54.0, egui::Color32::WHITE),
        (55.0, egui::Color32::WHITE),
        (64.0, egui::Color32::WHITE),
        (65.0, theme::DARK_TEXT),
        (84.0, theme::DARK_TEXT),
        (100.0, theme::DARK_TEXT),
    ] {
        for (page, required) in expected.iter().enumerate() {
            let ctx = egui::Context::default();
            app.page = page;
            app.appearance.controls_transparency = transparency;

            theme::apply(&ctx, &app.appearance);
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 6000.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    app.render(ctx);
                },
            );
            let mut found = Vec::new();
            for shape in &output.shapes {
                collect_text(&shape.shape, &mut found);
            }
            let common_text = [
                "MuMu + Alas",
                "自动重启控制台",
                "每 2 秒刷新状态",
                "退出界面",
                "就绪",
            ];
            for prefix in required.iter().copied().chain(common_text) {
                let matching: Vec<_> = found
                    .iter()
                    .filter(|(text, _)| text.starts_with(prefix))
                    .collect();
                assert!(
                    !matching.is_empty(),
                    "页面 {page} 缺少文字 {prefix}，透明度 {transparency}"
                );
                for (text, color) in matching {
                    assert_eq!(
                        *color, expected_color,
                        "页面 {page} 文字 {text}，透明度 {transparency}"
                    );
                }
            }
        }
    }
    std::fs::remove_file(path).unwrap();
}
