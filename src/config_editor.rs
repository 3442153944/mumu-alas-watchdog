//! 可视化配置编辑器：负责草稿、校验及保存，字段控件和标签独立维护。
use crate::config::{Settings, DEFAULT_CONFIG_TEXT};
use eframe::egui;
use serde_yaml::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub struct ConfigEditor {
    path: PathBuf,
    draft: Value,
    original: String,
    error: Option<String>,
    dirty: bool,
    pub reload_requested: bool,
}

fn merge_defaults(value: &mut Value, defaults: &Value) {
    if let (Some(map), Some(default_map)) = (value.as_mapping_mut(), defaults.as_mapping()) {
        for (key, default) in default_map {
            let entry = map.entry(key.clone()).or_insert_with(|| default.clone());
            merge_defaults(entry, default);
        }
    }
}

impl ConfigEditor {
    pub fn load(path: &Path) -> Self {
        let result = fs::read_to_string(path)
            .map_err(|e| e.to_string())
            .and_then(|text| {
                let mut draft: Value = serde_yaml::from_str(&text).map_err(|e| e.to_string())?;
                if !draft.is_mapping() {
                    return Err("配置顶层必须是映射".into());
                }
                merge_defaults(
                    &mut draft,
                    &serde_yaml::from_str::<Value>(DEFAULT_CONFIG_TEXT).unwrap(),
                );
                Ok((text, draft))
            });
        let (original, draft, error) = match result {
            Ok((text, draft)) => (text, draft, None),
            Err(e) => (String::new(), Value::Null, Some(e)),
        };
        Self {
            path: path.into(),
            original,
            draft,
            error,
            dirty: false,
            reload_requested: false,
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui, reload_busy: bool) -> Option<Settings> {
        let mut saved = None;
        ui.horizontal_wrapped(|ui| {
            ui.heading("配置");
            if ui
                .add_enabled(self.dirty && !reload_busy, egui::Button::new("保存配置"))
                .clicked()
            {
                match self.save() {
                    Ok(settings) => {
                        self.dirty = false;
                        self.error = None;
                        saved = Some(settings);
                    }
                    Err(e) => self.error = Some(e),
                }
            }
            if ui.button("放弃修改 / 重新读取").clicked() {
                *self = Self::load(&self.path);
            }
            if ui
                .add_enabled(
                    !self.dirty && !reload_busy,
                    egui::Button::new(if reload_busy {
                        "等待重载确认…"
                    } else {
                        "热重载已保存配置"
                    }),
                )
                .clicked()
            {
                self.reload_requested = true;
            }
            if self.dirty {
                ui.label("有未保存的修改");
            }
        });
        ui.small(self.path.display().to_string());
        ui.label("保存后自动请求后台热重载；不会执行启动时重启。原配置备份为 .yml.bak。");
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        ui.separator();
        egui::ScrollArea::vertical().show(ui, |ui| {
            let defaults: Value = serde_yaml::from_str(DEFAULT_CONFIG_TEXT).unwrap();
            for section in [
                "paths", "alas", "schedule", "health", "timeouts", "process", "monitor", "logging",
                "advanced",
            ] {
                egui::CollapsingHeader::new(label(section))
                    .default_open(matches!(section, "paths" | "alas" | "schedule" | "health"))
                    .show(ui, |ui| {
                        let Some(fields) = defaults[section].as_mapping() else {
                            return;
                        };
                        for (key, default) in fields {
                            let Some(key) = key.as_str() else {
                                continue;
                            };
                            let value = &mut self.draft[section][key];
                            let before = value.clone();
                            ui.push_id((section, key), |ui| {
                                egui::Frame::new()
                                    .fill(egui::Color32::TRANSPARENT)
                                    .corner_radius(6.0)
                                    .inner_margin(8.0)
                                    .show(ui, |ui| {
                                        let width = ui.available_width();
                                        if width < 760.0 {
                                            ui.label(label(key)).on_hover_text(key);
                                            field(ui, key, value, default);
                                        } else {
                                            ui.horizontal_top(|ui| {
                                                ui.add_sized(
                                                    [250.0, 36.0],
                                                    egui::Label::new(label(key))
                                                        .halign(egui::Align::Min),
                                                )
                                                .on_hover_text(key);
                                                let remaining = ui.available_width();
                                                ui.allocate_ui_with_layout(
                                                    egui::vec2(remaining, 36.0),
                                                    egui::Layout::top_down(egui::Align::Min),
                                                    |ui| {
                                                        field(ui, key, value, default);
                                                    },
                                                );
                                            });
                                        }
                                    });
                            });
                            self.dirty |= *value != before;
                            ui.add_space(8.0);
                        }
                    });
                ui.add_space(8.0);
            }
        });
        saved
    }

    fn save(&mut self) -> Result<Settings, String> {
        validate(&self.draft)?;
        let current = fs::read_to_string(&self.path).map_err(|e| e.to_string())?;
        if current != self.original {
            return Err("配置已被其他程序修改，请重新读取后再保存。".into());
        }
        let text = serde_yaml::to_string(&self.draft).map_err(|e| e.to_string())?;
        let temp = self
            .path
            .with_extension(format!("yml.{}.tmp", std::process::id()));
        fs::write(&temp, &text).map_err(|e| format!("写入临时配置失败：{e}"))?;
        let result = (|| {
            Settings::load(&temp)?;
            fs::write(self.path.with_extension("yml.bak"), &current)
                .map_err(|e| format!("备份失败：{e}"))?;
            fs::rename(&temp, &self.path).map_err(|e| format!("替换配置失败：{e}"))?;
            Settings::load(&self.path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        if result.is_ok() {
            self.original = text;
        }
        result
    }
}

pub fn validate(value: &Value) -> Result<(), String> {
    for key in ["alas_root", "mumu_manager", "adb_exe"] {
        let text = value["paths"][key].as_str().unwrap_or_default().trim();
        if text.is_empty() {
            return Err(format!("{}不能为空，可选择自动识别。", label(key)));
        }
        if !text.eq_ignore_ascii_case("auto") {
            let path = PathBuf::from(text);
            let path = if path.is_absolute() {
                path
            } else {
                crate::constants::wd_dir().join(path)
            };
            if (key == "alas_root" && !path.is_dir()) || (key != "alas_root" && !path.is_file()) {
                return Err(format!(
                    "{}不存在或类型不正确：{}",
                    label(key),
                    path.display()
                ));
            }
        }
    }
    if value["alas"]["config_names"]
        .as_sequence()
        .is_some_and(|v| v.is_empty())
    {
        return Err("请选择至少一个 Alas 配置，或启用自动识别。".into());
    }
    Ok(())
}

mod fields;
mod labels;
use fields::field;
use labels::label;
#[cfg(test)]
mod tests;
