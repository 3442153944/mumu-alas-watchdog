//! 只读状态轮询；只保留最新快照，隐藏窗口不会积压无限队列。
use crate::{config::Settings, constants, util, watchdog::Watchdog};
use serde_json::Value as Json;
use std::{
    sync::{Arc, Mutex, Weak},
    time::Duration,
};

pub(super) enum Update {
    Status(Json),
    Error(String),
}

#[derive(Default)]
pub(super) struct Updates(Arc<Mutex<Option<Update>>>);
impl Updates {
    pub(super) fn take(&self) -> Option<Update> {
        self.0.lock().ok()?.take()
    }
}
fn publish(target: &Weak<Mutex<Option<Update>>>, update: Update) -> bool {
    let Some(slot) = target.upgrade() else {
        return false;
    };
    if let Ok(mut value) = slot.lock() {
        *value = Some(update);
    }
    true
}

fn daemon_snapshot() -> Option<Json> {
    let pid = util::watcher_running_pid()?;
    let status = util::read_json(&constants::status_json())?;
    if status.pointer("/watchdog/pid").and_then(Json::as_u64) == Some(pid as u64)
        && status
            .pointer("/watchdog/next_restart")
            .is_some_and(|value| !value.is_null())
    {
        Some(status)
    } else {
        None
    }
}

pub(super) fn spawn(settings: Settings) -> Updates {
    let updates = Updates::default();
    let target = Arc::downgrade(&updates.0);
    std::thread::spawn(move || {
        let mut watchdog: Option<Watchdog> = None;
        loop {
            if target.strong_count() == 0 {
                break;
            }
            let snapshot = daemon_snapshot();
            let interval = if snapshot.is_some() { 2 } else { 5 };
            let payload = if let Some(status) = snapshot {
                // 后台已有快照时，不再额外持有一套巡检器和日志扫描状态。
                watchdog = None;
                status
            } else {
                if watchdog.is_none() {
                    match Watchdog::new(settings.clone()) {
                        Ok(value) => watchdog = Some(value),
                        Err(error) => {
                            publish(&target, Update::Error(error));
                            return;
                        }
                    }
                }
                let watcher = watchdog.as_mut().expect("巡检器已初始化");
                let texts = watcher.logwatch.scan();
                watcher.logwatch.analyze(&texts);
                watcher.refresh_emulator_runtime();
                let mut payload = watcher.collect_status();
                if payload
                    .pointer("/watchdog/next_restart")
                    .is_none_or(Json::is_null)
                {
                    let estimate = watcher.compute_next_restart(chrono::Local::now());
                    payload["watchdog"]["next_restart"] = serde_json::json!(format!(
                        "{}（估算）",
                        estimate.format("%Y-%m-%d %H:%M:%S")
                    ));
                }
                payload
            };
            if !publish(&target, Update::Status(payload)) {
                break;
            }
            std::thread::sleep(Duration::from_secs(interval));
        }
    });
    updates
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hidden_window_keeps_only_the_latest_snapshot() {
        let updates = Updates::default();
        let target = Arc::downgrade(&updates.0);
        for index in 0..10_000 {
            assert!(publish(
                &target,
                Update::Status(serde_json::json!({"sequence":index}))
            ));
        }
        match updates.take().unwrap() {
            Update::Status(value) => assert_eq!(value["sequence"], 9_999),
            _ => panic!("应为状态快照"),
        }
        assert!(updates.take().is_none());
        drop(updates);
        assert!(!publish(&target, Update::Error("已退出".into())));
    }
}
