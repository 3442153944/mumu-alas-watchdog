//! Small request/ack protocol using the existing control-file channel.
use crate::{config::Settings, constants};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    pub path: PathBuf,
    pub pid: u32,
}
#[derive(Serialize, Deserialize)]
pub struct Reply {
    pub id: String,
    pub pid: u32,
    pub ok: bool,
    pub message: String,
}
pub fn request_path() -> PathBuf {
    constants::wd_dir().join("reload-config.json")
}
pub fn reply_path() -> PathBuf {
    constants::wd_dir().join("reload-result.json")
}
pub fn read_settings(path: &Path) -> Result<Settings, String> {
    // Settings::load creates missing files: a reload must never do so.
    if !path.is_file() {
        return Err("配置文件不存在，保留原配置".into());
    }
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let raw: serde_yaml::Value = serde_yaml::from_str(&text).map_err(|e| e.to_string())?;
    if !raw.is_mapping() {
        return Err("配置顶层必须是映射".into());
    }
    validate_types(&raw)?;
    let mut merged: serde_yaml::Value =
        serde_yaml::from_str(crate::config::DEFAULT_CONFIG_TEXT).map_err(|e| e.to_string())?;
    for (section, values) in raw.as_mapping().unwrap() {
        if let (Some(target), Some(source)) = (
            merged.get_mut(section).and_then(|v| v.as_mapping_mut()),
            values.as_mapping(),
        ) {
            for (key, value) in source {
                target.insert(key.clone(), value.clone());
            }
        }
    }
    crate::config_editor::validate(&merged)?;
    let settings = Settings::from_text(path, &text, false)?;
    if !settings.alas_root.eq_ignore_ascii_case("auto") {
        let root = PathBuf::from(&settings.alas_root);
        let root = if root.is_absolute() {
            root
        } else {
            constants::wd_dir().join(root)
        };
        if !root.join("alas.py").is_file() || !root.join("gui.py").is_file() {
            return Err("Alas 根目录必须包含 alas.py 和 gui.py".into());
        }
    }
    Ok(settings)
}

fn validate_types(raw: &serde_yaml::Value) -> Result<(), String> {
    use serde_yaml::Value;
    let defaults: Value = serde_yaml::from_str(crate::config::DEFAULT_CONFIG_TEXT).unwrap();
    for (section, fields) in defaults.as_mapping().unwrap() {
        let Some(actual) = raw.get(section) else {
            continue;
        };
        let Some(actual) = actual.as_mapping() else {
            return Err(format!("{} 必须是配置分组", section.as_str().unwrap()));
        };
        for (key, default) in fields.as_mapping().unwrap() {
            let Some(value) = actual.get(key) else {
                continue;
            };
            let name = key.as_str().unwrap();
            let valid = match default {
                Value::Bool(_) => {
                    value.is_bool()
                        || value.as_str().is_some_and(|s| {
                            matches!(
                                s.to_ascii_lowercase().as_str(),
                                "true" | "false" | "yes" | "no" | "on" | "off" | "0" | "1"
                            )
                        })
                }
                Value::Number(_) => value
                    .as_f64()
                    .or_else(|| value.as_str().and_then(|s| s.parse::<f64>().ok()))
                    .is_some_and(|n| n.is_finite() && n >= 0.0),
                Value::Sequence(_) => {
                    let expected = crate::util::parse_hhmm_list(value);
                    let count = value.as_sequence().map(|v| v.len()).or_else(|| {
                        value.as_str().map(|s| {
                            s.split(|c: char| c == ',' || c == ';' || c.is_whitespace())
                                .filter(|v| !v.is_empty())
                                .count()
                        })
                    });
                    // Parsing sorts/deduplicates, so validate each supplied time separately.
                    count == Some(0)
                        || (!expected.is_empty()
                            && match value {
                                Value::Sequence(items) => items
                                    .iter()
                                    .all(|v| !crate::util::parse_hhmm_list(v).is_empty()),
                                Value::String(text) => text
                                    .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
                                    .filter(|v| !v.is_empty())
                                    .all(|s| {
                                        !crate::util::parse_hhmm_list(&Value::String(s.into()))
                                            .is_empty()
                                    }),
                                _ => false,
                            })
                }
                _ if name == "mumu_instance" => {
                    value.as_u64().is_some()
                        || value.as_str().is_some_and(|s| {
                            s.eq_ignore_ascii_case("auto") || s.parse::<u32>().is_ok()
                        })
                }
                _ if name == "config_names" => {
                    value.as_str().is_some_and(|s| !s.trim().is_empty())
                        || value.as_sequence().is_some_and(|items| {
                            !items.is_empty()
                                && items
                                    .iter()
                                    .all(|v| v.as_str().is_some_and(|s| !s.trim().is_empty()))
                        })
                }
                _ => value.is_string(),
            };
            if !valid {
                return Err(format!(
                    "配置项 {}.{name} 的格式不正确",
                    section.as_str().unwrap()
                ));
            }
        }
    }
    if raw["paths"]["launch_mode"]
        .as_str()
        .is_some_and(|s| !matches!(s, "electron" | "launcher"))
    {
        return Err("启动方式必须为 electron 或 launcher".into());
    }
    if raw["logging"]["level"].as_str().is_some_and(|s| {
        !matches!(
            s.to_ascii_uppercase().as_str(),
            "DEBUG" | "INFO" | "WARNING" | "WARN" | "ERROR"
        )
    }) {
        return Err("日志级别不正确".into());
    }
    Ok(())
}
pub fn request(path: &Path, pid: u32) -> Result<String, String> {
    let id = format!(
        "{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    );
    let request = Request {
        id: id.clone(),
        path: path.into(),
        pid,
    };
    write_json(&request_path(), &request)?;
    Ok(id)
}
pub fn write_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let result = (|| {
        fs::write(&temp, serde_json::to_vec(value).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        fs::rename(&temp, path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reload_rejects_bad_and_missing_files_without_creating_them() {
        let path = std::env::temp_dir().join(format!("muw-reload-{}.yml", std::process::id()));
        assert!(read_settings(&path).is_err());
        assert!(!path.exists());
        fs::write(&path, "paths: [invalid").unwrap();
        assert!(read_settings(&path).is_err());
        fs::remove_file(path).unwrap();
    }
    #[test]
    fn rejects_invalid_types_and_times_instead_of_silently_defaulting() {
        for text in [
            "health: {check_interval_seconds: bad}",
            "schedule: {restart_times: ['25:00']}",
            "schedule: {restart_times: ['04:00', 'bad']}",
            "health: {restart_alas_if_closed: perhaps}",
            "paths: {launch_mode: unknown}",
        ] {
            assert!(
                validate_types(&serde_yaml::from_str::<serde_yaml::Value>(text).unwrap()).is_err(),
                "{text}"
            );
        }
        assert!(validate_types(
            &serde_yaml::from_str::<serde_yaml::Value>(crate::config::DEFAULT_CONFIG_TEXT).unwrap()
        )
        .is_ok());
    }
}
