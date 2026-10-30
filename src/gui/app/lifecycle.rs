//! 窗口生命周期、托盘收起恢复与公共导航布局。
use super::*;
impl eframe::App for GuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if ctx
            .input(|i| i.viewport().minimized)
            .unwrap_or(false)
        {
            ctx.request_repaint_after(Duration::from_secs(2));
            return;
        }
        if !self.styled {
            theme::apply(ctx, &self.appearance);
            self.styled = true;
        }
        self.drain_updates();
        self.check_reload_reply();
        self.refresh_logs();
        // 状态数据每 2 秒更新一次，界面按 2 秒重绘即可；交互时 egui 会立即重绘。
        ctx.request_repaint_after(Duration::from_secs(2));
        self.render(ctx);
    }
}

impl GuiApp {
    /// 公共导航、消息和页面共用渲染入口，可在无窗口测试中整体检查。
    pub(super) fn render(&mut self, ctx: &egui::Context) {
        if let Some(texture) = &self.background {
            let rect = ctx.screen_rect();
            ctx.layer_painter(egui::LayerId::background()).rect_filled(
                rect,
                0.0,
                egui::Color32::from_rgb(219, 225, 231),
            );
            ctx.layer_painter(egui::LayerId::background()).image(
                texture.id(),
                rect,
                background::uv(rect.size(), texture.size_vec2()),
                theme::with_transparency(
                    egui::Color32::WHITE,
                    self.appearance.background_transparency,
                ),
            );
        }

        egui::TopBottomPanel::top("navigation")
            .frame(
                egui::Frame::new()
                    .fill(theme::surface(&self.appearance))
                    .inner_margin(20.0),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("MuMu + Alas");
                    ui.label("自动重启控制台");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button("退出界面")
                            .on_hover_text("关闭控制台窗口，后台看门狗与托盘图标继续运行")
                            .clicked()
                        {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                        ui.small("每 2 秒刷新状态");
                    });
                });
                ui.add_space(16.0);
                ui.horizontal(|ui| {
                    for (index, title) in
                        ["运行概览", "配置", "运行日志", "外观"].iter().enumerate()
                    {
                        ui.selectable_value(&mut self.page, index, *title);
                    }
                });
            });
        egui::TopBottomPanel::bottom("message")
            .frame(
                egui::Frame::new()
                    .fill(theme::surface(&self.appearance))
                    .inner_margin(8.0),
            )
            .show(ctx, |ui| {
                ui.add_space(6.0);
                ui.label(&self.message);
                ui.add_space(6.0);
            });
        let page_appearance = self.appearance.clone();
        theme::content_panel(ctx, &page_appearance, |ui| self.show_page(ui, ctx));
    }
}
