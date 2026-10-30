//! 外观配置的持久化模型，不依赖界面渲染和控件样式。
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub background_path: Option<PathBuf>,
    pub background_transparency: f32,
    pub controls_transparency: f32,
}
impl Default for Appearance {
    fn default() -> Self {
        Self {
            background_path: None,
            background_transparency: 0.0,
            controls_transparency: 55.0,
        }
    }
}
impl Appearance {
    pub fn path() -> PathBuf {
        crate::constants::wd_dir().join("gui-appearance.json")
    }
    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let mut value: Self =
            serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("外观设置格式错误：{e}"))?;
        value.background_transparency = normalized(value.background_transparency, 0.0);
        value.controls_transparency = normalized(value.controls_transparency, 55.0);
        Ok(value)
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        crate::reload::write_json(path, self)
    }
}
pub(crate) fn normalized(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 100.0)
    } else {
        fallback
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferences_round_trip_without_changing_watchdog_config() {
        let path = std::env::temp_dir().join(format!("muw-appearance-{}.json", std::process::id()));
        let value = Appearance {
            background_path: Some(PathBuf::from("C:/图片/我的背景.png")),
            background_transparency: 20.0,
            controls_transparency: 70.0,
        };
        value.save(&path).unwrap();
        assert_eq!(Appearance::load(&path).unwrap(), value);
        std::fs::remove_file(path).unwrap();
    }
}
