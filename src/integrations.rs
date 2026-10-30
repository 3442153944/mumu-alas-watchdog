//! 系统集成：开机自启快捷方式、计划任务监督者（每 5 分钟 --ensure）。

use std::path::Path;

use crate::constants;
use crate::util;

fn powershell(script: &str) -> util::CmdResult {
    util::run_cmd(
        "powershell",
        &["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script],
        std::time::Duration::from_secs(60),
    )
}

fn ps_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

pub fn install_autostart() -> Result<(), String> {
    let appdata = std::env::var("APPDATA").map_err(|_| "找不到 APPDATA 环境变量".to_string())?;
    let shortcut = Path::new(&appdata)
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join("Startup")
        .join(constants::AUTOSTART_SHORTCUT_NAME);
    let script = format!(
        "$ws = New-Object -ComObject WScript.Shell; $lnk = $ws.CreateShortcut({}); $lnk.TargetPath = {}; $lnk.Arguments = '--start'; $lnk.WorkingDirectory = {}; $lnk.WindowStyle = 7; $lnk.Description = 'MuMu + Alas watchdog'; $lnk.Save()",
        ps_quote(&shortcut.to_string_lossy()),
        ps_quote(&constants::app_path().to_string_lossy()),
        ps_quote(&constants::wd_dir().to_string_lossy()),
    );
    let result = powershell(&script);
    if result.rc != 0 || !shortcut.exists() {
        return Err(format!(
            "创建开机自启快捷方式失败: {}",
            result.combined()
        ));
    }
    Ok(())
}

pub fn uninstall_autostart() -> Result<(), String> {
    let appdata = std::env::var("APPDATA").map_err(|_| "找不到 APPDATA 环境变量".to_string())?;
    let shortcut = Path::new(&appdata)
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join("Startup")
        .join(constants::AUTOSTART_SHORTCUT_NAME);
    if shortcut.exists() {
        std::fs::remove_file(&shortcut)
            .map_err(|err| format!("删除开机自启快捷方式失败: {err}"))?;
    }
    Ok(())
}

pub fn install_supervisor() -> Result<(), String> {
    let exe = constants::app_path();
    let command = format!("\"{}\" --ensure", exe.display());
    let result = util::run_cmd(
        "schtasks",
        &[
            "/Create",
            "/F",
            "/SC",
            "MINUTE",
            "/MO",
            "5",
            "/TN",
            constants::SUPERVISOR_TASK_NAME,
            "/TR",
            &command,
            "/RL",
            "LIMITED",
        ],
        std::time::Duration::from_secs(60),
    );
    if result.rc != 0 {
        return Err(format!("创建监督者计划任务失败: {}", result.combined()));
    }
    if !crate::watchdog::supervisor_installed() {
        return Err("计划任务创建后校验失败".to_string());
    }
    Ok(())
}

pub fn uninstall_supervisor() -> Result<(), String> {
    let result = util::run_cmd(
        "schtasks",
        &["/Delete", "/F", "/TN", constants::SUPERVISOR_TASK_NAME],
        std::time::Duration::from_secs(60),
    );
    if result.rc != 0 && crate::watchdog::supervisor_installed() {
        return Err(format!("删除监督者计划任务失败: {}", result.combined()));
    }
    Ok(())
}

pub fn supervisor_installed() -> bool {
    crate::watchdog::supervisor_installed()
}
