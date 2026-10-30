//! 配置字段控件，颜色和文字继承统一主题。
use super::*;
pub(super) fn field(ui: &mut egui::Ui, key: &str, value: &mut Value, default: &Value) {
    if key == "restart_times" {
        if !value.is_sequence() {
            *value = Value::Sequence(
                crate::util::parse_hhmm_list(value)
                    .iter()
                    .map(|(h, m)| Value::String(format!("{h:02}:{m:02}")))
                    .collect(),
            );
        }
        let items = value.as_sequence_mut().unwrap();
        ui.vertical(|ui| {
            ui.small("留空按间隔重启；添加时间后改为每天定时重启。");
            let mut remove = None;
            for (i, item) in items.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    let text = item.as_str().unwrap_or("04:00");
                    let parts: Vec<_> = text.split(':').collect();
                    let mut hour: u32 = parts.first().and_then(|s| s.parse().ok()).unwrap_or(4);
                    let mut minute: u32 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
                    ui.add(egui::DragValue::new(&mut hour).range(0..=23).suffix(" 时"));
                    ui.add(
                        egui::DragValue::new(&mut minute)
                            .range(0..=59)
                            .suffix(" 分"),
                    );
                    *item = Value::String(format!("{hour:02}:{minute:02}"));
                    if ui
                        .add_sized([72.0, 36.0], egui::Button::new("移除"))
                        .clicked()
                    {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                items.remove(i);
            }
            if ui.button("添加重启时间").clicked() {
                items.push(Value::String("04:00".into()));
            }
        });
    } else if matches!(default, Value::Bool(_)) {
        let mut flag = value
            .as_bool()
            .or_else(|| {
                value.as_str().map(|s| {
                    matches!(
                        s.trim().to_ascii_lowercase().as_str(),
                        "1" | "true" | "yes" | "on"
                    )
                })
            })
            .unwrap_or(default.as_bool().unwrap_or(false));
        ui.checkbox(&mut flag, "启用");
        *value = Value::Bool(flag);
    } else if default.is_number() {
        let mut number = value
            .as_f64()
            .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
            .unwrap_or(default.as_f64().unwrap_or(0.0));
        let min = match key {
            "restart_interval_hours" => 0.1,
            "check_interval_seconds" | "mumu_cmd_timeout_seconds" => 20.0,
            "retry_delay_seconds" | "status_log_seconds" => 30.0,
            "adb_fail_threshold" => 2.0,
            "alas_restart_cooldown_seconds" => 60.0,
            "task_stuck_cooldown_seconds" => 300.0,
            "kill_alas_seconds" => 5.0,
            "keep_days" => 1.0,
            _ => 0.0,
        };
        let fractional = matches!(key, "restart_interval_hours" | "mumu_memory_limit_gb");
        let response = ui.add(
            egui::DragValue::new(&mut number)
                .range(min..=1_000_000.0)
                .speed(if fractional { 0.1 } else { 1.0 })
                .max_decimals(if fractional { 1 } else { 0 }),
        );
        if response.changed() {
            *value = if fractional {
                serde_yaml::to_value(number).unwrap()
            } else {
                Value::Number((number as u64).into())
            };
        }
    } else if matches!(key, "launch_mode" | "level") {
        let mut text = value.as_str().unwrap_or_default().to_owned();
        let options: &[&str] = if key == "launch_mode" {
            &["electron", "launcher"]
        } else {
            &["DEBUG", "INFO", "WARNING", "ERROR"]
        };
        egui::ComboBox::from_id_salt(key)
            .selected_text(&text)
            .show_ui(ui, |ui| {
                for option in options {
                    ui.selectable_value(&mut text, (*option).into(), *option);
                }
            });
        *value = Value::String(text);
    } else if key == "mumu_instance" {
        let mut auto = value
            .as_str()
            .is_some_and(|s| s.eq_ignore_ascii_case("auto"));
        let mut index = value
            .as_u64()
            .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
            .unwrap_or(0);
        ui.horizontal(|ui| {
            ui.checkbox(&mut auto, "自动识别");
            ui.add_enabled(!auto, egui::DragValue::new(&mut index).range(0..=999));
        });
        *value = if auto {
            Value::String("auto".into())
        } else {
            Value::Number(index.into())
        };
    } else if key == "config_names" {
        let mut auto = value
            .as_str()
            .is_some_and(|s| s.eq_ignore_ascii_case("auto"));
        ui.vertical(|ui| {
            ui.checkbox(&mut auto, "自动识别 Alas 配置");
            if auto {
                *value = Value::String("auto".into());
            } else {
                if !value.is_sequence() {
                    *value = Value::Sequence(
                        value
                            .as_str()
                            .unwrap_or_default()
                            .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
                            .filter(|s| !s.is_empty())
                            .map(|s| Value::String(s.into()))
                            .collect(),
                    );
                }
                let names = value.as_sequence_mut().unwrap();
                for name in names.iter() {
                    ui.label(name.as_str().unwrap_or_default());
                }
                ui.horizontal(|ui| {
                    if ui.button("选择 Alas 配置 JSON（可多选）").clicked() {
                        if let Some(files) = rfd::FileDialog::new()
                            .add_filter("Alas 配置", &["json"])
                            .pick_files()
                        {
                            *names = files
                                .iter()
                                .filter_map(|f| {
                                    f.file_stem()?.to_str().map(|s| Value::String(s.into()))
                                })
                                .collect();
                        }
                    }
                    if ui.button("清空").clicked() {
                        names.clear();
                    }
                });
            }
        });
    } else {
        let mut text = value.as_str().unwrap_or_default().to_owned();
        ui.horizontal(|ui| {
            let button_width = 78.0;
            let count = if key == "dir" { 1.0 } else { 2.0 };
            let input_width = (ui.available_width()
                - count * (button_width + ui.spacing().item_spacing.x))
                .max(80.0);
            ui.add_sized(
                [input_width, 36.0],
                egui::TextEdit::singleline(&mut text).desired_width(input_width),
            );
            if ui
                .add_sized([button_width, 36.0], egui::Button::new("选择…"))
                .clicked()
            {
                let dialog = rfd::FileDialog::new();
                let path = if matches!(key, "alas_root" | "dir") {
                    dialog.pick_folder()
                } else {
                    dialog.add_filter("可执行文件", &["exe"]).pick_file()
                };
                if let Some(path) = path {
                    text = path.to_string_lossy().into_owned();
                }
            }
            if key != "dir"
                && ui
                    .add_sized([button_width, 36.0], egui::Button::new("自动"))
                    .clicked()
            {
                text = "auto".into();
            }
        });
        *value = Value::String(text);
    }
}
