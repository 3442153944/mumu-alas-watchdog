//! 环境布局探测：Alas 根目录、MuMuManager、实例编号、ADB、配置名、WebUI 端口。

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value as JsonValue;
use serde_yaml::Value as YamlValue;

use crate::config::Settings;
use crate::constants;
use crate::emulator;
use crate::util;
use crate::win32;

#[derive(Debug, Clone)]
pub struct Layout {
    pub alas_root: PathBuf,
    pub mumu_manager: PathBuf,
    pub adb_exe: PathBuf,
    pub electron_exe: PathBuf,
    pub launcher_exe: PathBuf,
    pub toolkit_python: PathBuf,
    pub alas_log_dir: PathBuf,
    pub webui_port: u16,
    pub serial: String,
    pub instance: u32,
    pub instance_name: Option<String>,
    pub configs: Vec<String>,
    pub launch_mode: String,
    pub serial_config: String,
    pub emulator_name: String,
}

impl Layout {
    pub fn launch_exe(&self) -> &Path {
        if self.launch_mode == "launcher" {
            &self.launcher_exe
        } else {
            &self.electron_exe
        }
    }
}

fn resolve_configured(value: &str) -> Option<PathBuf> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("auto") {
        return None;
    }
    let path = PathBuf::from(trimmed);
    if path.is_absolute() {
        Some(path)
    } else {
        Some(constants::wd_dir().join(path))
    }
}

fn read_yaml(path: &Path) -> Option<YamlValue> {
    let text = fs::read_to_string(path).ok()?;
    serde_yaml::from_str(&text).ok()
}

fn read_json(path: &Path) -> Option<JsonValue> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn yaml_string(root: &YamlValue, path: &[&str]) -> Option<String> {
    let mut current = root;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str().map(|text| text.trim().to_string())
}

fn yaml_u64(root: &YamlValue, path: &[&str]) -> Option<u64> {
    let mut current = root;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_u64()
}

fn detect_alas_root(configured: &str) -> Result<PathBuf, String> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(path) = resolve_configured(configured) {
        candidates.push(path);
    }
    let project_root = constants::wd_dir();
    candidates.push(project_root.parent().unwrap_or(project_root).join("AzurLaneAutoScript"));
    candidates.push(project_root.join("AzurLaneAutoScript"));
    if let Some(parent) = project_root.parent() {
        if let Some(grand) = parent.parent() {
            candidates.push(grand.join("AzurLaneAutoScript"));
        }
    }
    // 从运行中的 Alas Electron 进程反推：<root>/toolkit/webapp/alas.exe
    for entry in win32::list_processes() {
        if !entry.name.eq_ignore_ascii_case("alas.exe") {
            continue;
        }
        let Some(exe) = win32::proc_exe(entry.pid) else {
            continue;
        };
        let parts: Vec<String> = exe
            .components()
            .map(|part| part.as_os_str().to_string_lossy().to_ascii_lowercase())
            .collect();
        if parts.iter().any(|part| part == "toolkit") && parts.iter().any(|part| part == "webapp") {
            if let Some(root) = exe.ancestors().nth(3) {
                candidates.push(root.to_path_buf());
            }
        }
    }
    for candidate in candidates {
        let resolved = candidate.canonicalize().unwrap_or(candidate);
        if resolved.join("alas.py").exists() && resolved.join("gui.py").exists() {
            return Ok(util::strip_verbatim(&resolved));
        }
    }
    Err("找不到 Alas 目录：请在 config.yml 的 paths.alas_root 填写 AzurLaneAutoScript 的完整路径".to_string())
}

fn detect_mumu_manager(configured: &str, emulator_path: &str) -> Result<PathBuf, String> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(path) = resolve_configured(configured) {
        candidates.push(path);
    }
    if !emulator_path.trim().is_empty() {
        let path = PathBuf::from(emulator_path.trim());
        candidates.push(path.parent().unwrap_or(Path::new("")).join("MuMuManager.exe"));
        if let Some(parent) = path.parent().and_then(|parent| parent.parent()) {
            candidates.push(parent.join("shell").join("MuMuManager.exe"));
        }
    }
    for entry in win32::list_processes() {
        if !(entry.name.eq_ignore_ascii_case("MuMuNxMain.exe")
            || entry.name.eq_ignore_ascii_case("MuMuPlayer.exe"))
        {
            continue;
        }
        if let Some(exe) = win32::proc_exe(entry.pid) {
            if let Some(dir) = exe.parent() {
                candidates.push(dir.join("MuMuManager.exe"));
                candidates.push(dir.join("shell").join("MuMuManager.exe"));
            }
        }
    }
    // 常见安装目录扫描
    for base in [
        r"C:\Program Files\Netease",
        r"D:\Program Files\Netease",
        r"E:\Program Files\Netease",
        r"F:\Program Files\Netease",
        r"C:\Program Files",
    ] {
        let dir = Path::new(base);
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if name.starts_with("mumuplayer") {
                    candidates.push(path.join("nx_main").join("MuMuManager.exe"));
                    candidates.push(path.join("shell").join("MuMuManager.exe"));
                }
            }
        }
    }
    for candidate in candidates {
        if candidate.exists() {
            let resolved = candidate.canonicalize().unwrap_or(candidate);
            return Ok(util::strip_verbatim(&resolved));
        }
    }
    Err("找不到 MuMuManager.exe：请在 config.yml 的 paths.mumu_manager 填写完整路径".to_string())
}

fn serial_port(serial: &str) -> Option<u16> {
    serial.rsplit(':').next()?.trim().parse::<u16>().ok()
}

fn detect_instance(
    configured: Option<u32>,
    serial_config: &str,
    emulator_name: &str,
    manager: &Path,
) -> (u32, Option<String>) {
    let mut index = configured;
    let all_info = emulator::query_info_all(manager, std::time::Duration::from_secs(25));
    if index.is_none() {
        if let Some(port) = serial_port(serial_config) {
            for (key, item) in &all_info {
                if item.get("adb_port").and_then(|value| value.as_u64()) == Some(port as u64) {
                    index = key.parse::<u32>().ok();
                    break;
                }
            }
        }
    }
    if index.is_none() {
        if let Some(suffix) = emulator_name.rsplit('-').next() {
            index = suffix.trim().parse::<u32>().ok();
        }
    }
    let instance = index.unwrap_or(0);
    let instance_name = all_info
        .get(&instance.to_string())
        .and_then(|item| item.get("name"))
        .and_then(|value| value.as_str())
        .map(|text| text.to_string())
        .or_else(|| {
            if emulator_name.is_empty() {
                None
            } else {
                Some(emulator_name.to_string())
            }
        });
    (instance, instance_name)
}

fn detect_adb(configured: &str, alas_root: &Path, deploy: Option<&YamlValue>) -> Result<PathBuf, String> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(path) = resolve_configured(configured) {
        candidates.push(path);
    } else if let Some(deploy) = deploy {
        if let Some(value) = yaml_string(deploy, &["Deploy", "Adb", "AdbExecutable"]) {
            if !value.eq_ignore_ascii_case("null") {
                let path = PathBuf::from(&value);
                candidates.push(if path.is_absolute() {
                    path
                } else {
                    alas_root.join(path)
                });
            }
        }
    }
    candidates.push(
        alas_root
            .join("toolkit")
            .join("Lib")
            .join("site-packages")
            .join("adbutils")
            .join("binaries")
            .join("adb.exe"),
    );
    for candidate in candidates {
        if candidate.exists() {
            let resolved = candidate.canonicalize().unwrap_or(candidate);
            return Ok(util::strip_verbatim(&resolved));
        }
    }
    Err("找不到 adb.exe：请在 config.yml 的 paths.adb_exe 填写完整路径".to_string())
}

fn detect_config_names(configured: &Option<Vec<String>>, alas_root: &Path) -> Result<Vec<String>, String> {
    if let Some(names) = configured {
        if !names.is_empty() {
            return Ok(names.clone());
        }
    }
    let config_dir = alas_root.join("config");
    let mut names: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(&config_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if stem.starts_with("template") || stem == "deploy" {
                continue;
            }
            names.push(stem.to_string());
        }
    }
    names.sort();
    if names.is_empty() {
        return Err(format!("在 {} 里找不到 Alas 配置文件（<配置名>.json）", config_dir.display()));
    }
    Ok(names)
}

pub fn detect_layout(settings: &Settings) -> Result<Layout, String> {
    let alas_root = detect_alas_root(&settings.alas_root)?;
    let deploy = read_yaml(&alas_root.join("config").join("deploy.yaml"));
    let alas_cfg = read_json(&alas_root.join("config").join("alas.json"));
    let serial_config = alas_cfg
        .as_ref()
        .and_then(|value| value.pointer("/Alas/Emulator/Serial"))
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    let emulator_name = alas_cfg
        .as_ref()
        .and_then(|value| value.pointer("/Alas/EmulatorInfo/name"))
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    let emulator_path = alas_cfg
        .as_ref()
        .and_then(|value| value.pointer("/Alas/EmulatorInfo/path"))
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();

    let mumu_manager = detect_mumu_manager(&settings.mumu_manager, &emulator_path)?;
    let (instance, instance_name) = detect_instance(
        settings.mumu_instance,
        &serial_config,
        &emulator_name,
        &mumu_manager,
    );
    let serial = if serial_config.is_empty() {
        format!("127.0.0.1:{}", 16384 + instance * 32)
    } else {
        serial_config.clone()
    };
    let adb_exe = detect_adb(&settings.adb_exe, &alas_root, deploy.as_ref())?;

    let webui_port = deploy
        .as_ref()
        .and_then(|value| yaml_u64(value, &["Deploy", "Webui", "WebuiPort"]))
        .map(|port| port as u16)
        .unwrap_or(22267);

    let configs = detect_config_names(&settings.config_names, &alas_root)?;

    let electron_exe = alas_root.join("toolkit").join("webapp").join("alas.exe");
    let launcher_exe = alas_root.join("Alas.exe");
    let mut launch_mode = settings.launch_mode.clone();
    if launch_mode == "launcher" && !launcher_exe.exists() {
        launch_mode = "electron".to_string();
    }
    if launch_mode == "electron" && !electron_exe.exists() {
        if launcher_exe.exists() {
            launch_mode = "launcher".to_string();
        } else {
            return Err(format!("找不到 Alas 启动文件: {}", electron_exe.display()));
        }
    }
    if !alas_root.join("gui.py").exists() {
        return Err(format!("Alas 目录不完整（没有 gui.py）: {}", alas_root.display()));
    }

    Ok(Layout {
        electron_exe,
        launcher_exe,
        toolkit_python: alas_root.join("toolkit").join("python.exe"),
        alas_log_dir: alas_root.join("log"),
        webui_port,
        serial,
        instance,
        instance_name,
        configs,
        launch_mode,
        serial_config,
        emulator_name,
        alas_root,
        mumu_manager,
        adb_exe,
    })
}

pub fn resolve_path_under(root: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        root.join(path)
    }
}

pub fn _unused(_: &Layout) {
    let _ = util::now_text();
}
