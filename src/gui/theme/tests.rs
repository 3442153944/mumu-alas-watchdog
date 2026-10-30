use super::*;
#[test]
fn transparency_has_clear_endpoints_and_defaults() {
    assert_eq!(alpha(0.0), 255);
    assert_eq!(alpha(100.0), 0);
    assert_eq!(alpha(-10.0), 255);
    assert_eq!(alpha(110.0), 0);
    assert_eq!(Appearance::default().background_transparency, 0.0);
    assert_eq!(Appearance::default().controls_transparency, 55.0);
}
#[test]
fn intermediate_transparency_does_not_brighten_beyond_either_layer() {
    let background = egui::Color32::from_rgb(224, 220, 212);
    let foreground = PANEL_BLUE;
    for transparency in [0.0, 16.0, 55.0, 84.0, 100.0] {
        let appearance = Appearance {
            controls_transparency: transparency,
            ..Default::default()
        };
        let surface = surface(&appearance);
        let blended = background.blend(surface);
        for channel in 0..3 {
            assert!(surface[channel] <= surface.a());
            assert!(blended[channel] >= background[channel].min(foreground[channel]));
            assert!(blended[channel] <= background[channel].max(foreground[channel]));
        }
    }
}
#[test]
fn blue_theme_inverts_all_control_states_at_transparency_boundary() {
    let ctx = egui::Context::default();
    for (transparency, expected) in [
        (0.0, egui::Color32::WHITE),
        (54.0, egui::Color32::WHITE),
        (55.0, egui::Color32::WHITE),
        (64.0, egui::Color32::WHITE),
        (65.0, DARK_TEXT),
        (84.0, DARK_TEXT),
        (100.0, DARK_TEXT),
    ] {
        apply(
            &ctx,
            &Appearance {
                controls_transparency: transparency,
                ..Default::default()
            },
        );
        let style = ctx.style();
        assert_eq!(style.visuals.override_text_color, None);
        assert_eq!(
            style.visuals.widgets.noninteractive.fg_stroke.color,
            expected
        );
        assert_eq!(style.visuals.selection.stroke.color, expected);
        for widget in [
            &style.visuals.widgets.inactive,
            &style.visuals.widgets.hovered,
            &style.visuals.widgets.active,
            &style.visuals.widgets.open,
        ] {
            assert_eq!(
                widget.fg_stroke.color, expected,
                "控件透明度 {transparency}"
            );
        }
    }
}
#[test]
fn common_panel_fills_available_width_at_small_and_large_sizes() {
    let ctx = egui::Context::default();
    for width in [680.0, 1020.0, 1920.0] {
        let mut content_width = 0.0;
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 700.0),
                )),
                ..Default::default()
            },
            |ctx| {
                content_panel(ctx, &Appearance::default(), |ui| {
                    content_width = ui.available_width();
                    ui.label("短内容也应铺满宽度");
                });
            },
        );
        assert!(
            (content_width - (width - 48.0)).abs() < 1.0,
            "窗口宽度 {width}，内容宽度 {content_width}"
        );
    }
}
#[test]
fn slider_rail_and_handle_use_requested_blue() {
    let ctx = egui::Context::default();
    let mut value = 54.0;
    let output = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            transparency_slider(ui, &mut value);
        });
    });
    fn visit(shape: &egui::Shape, blue_rails: &mut usize, blue_handles: &mut usize) {
        match shape {
            egui::Shape::Rect(rect) if rect.fill == PANEL_BLUE => *blue_rails += 1,
            egui::Shape::Circle(circle) if circle.fill == PANEL_BLUE => *blue_handles += 1,
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    visit(shape, blue_rails, blue_handles);
                }
            }
            _ => {}
        }
    }
    let (mut rails, mut handles) = (0, 0);
    for shape in &output.shapes {
        visit(&shape.shape, &mut rails, &mut handles);
    }
    assert!(rails >= 2, "滑轨和已选区间应使用 #0096FA");
    assert!(handles >= 1, "滑块应使用 #0096FA");
}
