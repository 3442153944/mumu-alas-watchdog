//! 界面状态与操作编排，不定义页面配色或排版。
use super::{
    background, theme,
    worker::{self, Update},
};
use crate::{
    appearance::Appearance, config::Settings, config_editor::ConfigEditor, constants, util,
};
use eframe::egui;
use serde_json::Value as Json;
use std::time::{Duration, Instant};
mod lifecycle;
mod pages;
mod status;
#[cfg(test)]
mod tests;
pub(super) struct GuiApp {
    settings: Settings,
    editor: ConfigEditor,
    page: usize,
    styled: bool,
    receiver: worker::Updates,
    status: Json,
    logs: String,
    message: String,
    last_log_refresh: Instant,
    reload_pending: Option<(String, u32, Instant)>,
    background: Option<egui::TextureHandle>,
    appearance: Appearance,
    saved_appearance: Appearance,
}
impl GuiApp {
    pub(super) fn create(settings: Settings, ctx: &egui::Context) -> Result<Self, String> {
        let mut app = GuiApp::new(settings);
        match Appearance::load(&Appearance::path()) {
            Ok(value) => app.appearance = value,
            Err(error) => app.message = format!("{error}；暂用默认外观"),
        }
        app.saved_appearance = app.appearance.clone();
        match background::load_background(ctx, app.appearance.background_path.as_deref()) {
            Ok(texture) => app.background = Some(texture),
            Err(error) => {
                app.background = Some(background::load_background(ctx, None)?);
                app.message = format!("{error}；暂用内置背景，可在外观页重新选择");
            }
        }
        Ok(app)
    }
    pub(super) fn new(settings: Settings) -> Self {
        let receiver = worker::spawn(settings.clone());
        GuiApp {
            editor: ConfigEditor::load(&settings.path),
            page: if settings.created { 1 } else { 0 },
            styled: false,
            settings,
            receiver,
            status: Json::Null,
            logs: String::new(),
            message: "就绪：看门狗未运行时可查看状态；点“启动看门狗”后后台常驻".to_string(),
            last_log_refresh: Instant::now() - Duration::from_secs(10),
            reload_pending: None,
            background: None,
            appearance: Appearance::default(),
            saved_appearance: Appearance::default(),
        }
    }
    fn spawn_self(&mut self, args: &[&str], description: &str) {
        let mut args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        args.extend([
            "--config".to_owned(),
            self.settings.path.to_string_lossy().into_owned(),
        ]);
        match util::spawn_process(
            constants::app_path(),
            &args,
            Some(constants::wd_dir()),
            false,
            true,
            true,
        ) {
            Ok(pid) => self.message = format!("已触发：{description}（PID {pid}）"),
            Err(err) => self.message = format!("触发失败：{err}"),
        }
    }
    fn request_reload(&mut self) {
        if self.reload_pending.is_some() {
            return;
        }
        if let Some(pid) = util::watcher_running_pid() {
            match crate::reload::request(&self.settings.path, pid) {
                Ok(id) => {
                    self.reload_pending = Some((id, pid, Instant::now()));
                    self.message = "已请求热重载，等待后台完成当前操作并确认…".into();
                }
                Err(e) => self.message = format!("热重载请求失败：{e}"),
            }
        } else {
            match crate::reload::read_settings(&self.settings.path) {
                Ok(settings) => {
                    let replacement = GuiApp::new(settings);
                    self.settings = replacement.settings;
                    self.receiver = replacement.receiver;
                    self.status = Json::Null;
                    self.message = "配置已加载；看门狗未运行，下次启动使用新配置".into();
                }
                Err(e) => self.message = format!("重新加载失败：{e}"),
            }
        }
    }
    fn check_reload_reply(&mut self) {
        let Some((id, pid, started)) = &self.reload_pending else {
            return;
        };
        let reply = std::fs::read(crate::reload::reply_path())
            .ok()
            .and_then(|bytes| serde_json::from_slice::<crate::reload::Reply>(&bytes).ok());
        if let Some(reply) = reply.filter(|r| r.id == *id && r.pid == *pid) {
            if reply.ok {
                if let Ok(settings) = crate::reload::read_settings(&self.settings.path) {
                    let replacement = GuiApp::new(settings);
                    self.settings = replacement.settings;
                    self.receiver = replacement.receiver;
                    self.status = Json::Null;
                }
            }
            self.message = if reply.ok {
                reply.message
            } else {
                format!("热重载失败，后台保留原配置：{}", reply.message)
            };
            self.reload_pending = None;
        } else if util::watcher_running_pid() != Some(*pid) {
            self.message = "看门狗已退出或进程发生变化，请重新请求热重载".into();
            self.reload_pending = None;
        } else if started.elapsed() > Duration::from_secs(120) {
            self.message = "尚未收到热重载确认：后台可能仍在执行操作，或运行的是不支持热重载的旧版本。可稍后重试。".into();
            self.reload_pending = None;
        }
    }
    fn refresh_logs(&mut self) {
        // 不在日志页时不读日志文件；收起窗口时 update 会整体跳过。
        if self.page != 2 {
            return;
        }
        if self.last_log_refresh.elapsed() < Duration::from_secs(2) {
            return;
        }
        self.last_log_refresh = Instant::now();
        if let Some(path) = util::newest_log_file(&self.settings.log_dir) {
            self.logs = util::tail_lines(&path, 200, 128 * 1024).join("\n");
        }
    }
    fn drain_updates(&mut self) {
        if let Some(update) = self.receiver.take() {
            match update {
                Update::Status(payload) => self.status = payload,
                Update::Error(err) => self.message = err,
            }
        }
    }
}
