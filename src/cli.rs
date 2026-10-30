//! CLI 入口与子命令。

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;

use crate::config::Settings;
use crate::constants;
use crate::logger;
use crate::outln;
use crate::util;
use crate::watchdog::{self, Watchdog};

#[derive(Parser, Debug)]
#[command(
    name = "MuMuAlasWatchdog",
    version,
    about = "MuMu 模拟器 + Alas 自动重启看门狗（Rust + egui）",
    long_about = None
)]
struct Cli {
    /// 打开 GUI 控制台（默认）
    #[arg(long)]
    gui: bool,
    /// 前台 CLI 守护模式（控制台日志，调试用）
    #[arg(long)]
    cli: bool,
    /// 内部使用：后台守护进程
    #[arg(long, hide = true)]
    daemon: bool,
    /// 后台启动看门狗（无窗口）
    #[arg(long)]
    start: bool,
    /// 停止正在运行的看门狗
    #[arg(long)]
    stop: bool,
    /// 只读检查当前状态
    #[arg(long)]
    check: bool,
    /// 立即执行一次完整重启
    #[arg(long = "restart-now")]
    restart_now: bool,
    /// 只重启 Alas（不重启模拟器）
    #[arg(long = "restart-alas-now")]
    restart_alas_now: bool,
    /// 实测指定 MuMu 实例的启动/关闭（不影响 Alas）
    #[arg(long = "test-emulator", value_name = "INDEX")]
    test_emulator: Option<u32>,
    /// 配合 --test-emulator：测试后不关闭实例
    #[arg(long = "keep-running")]
    keep_running: bool,
    /// 安装开机自启
    #[arg(long = "install-autostart")]
    install_autostart: bool,
    /// 移除开机自启
    #[arg(long = "uninstall-autostart")]
    uninstall_autostart: bool,
    /// 安装监督者计划任务（每 5 分钟 --ensure）
    #[arg(long = "install-supervisor")]
    install_supervisor: bool,
    /// 移除监督者计划任务
    #[arg(long = "uninstall-supervisor")]
    uninstall_supervisor: bool,
    /// 内部使用：若看门狗未运行则拉起（监督者调用）
    #[arg(long, hide = true)]
    ensure: bool,
    /// 配置文件路径
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
}

fn load_settings(config: &Option<PathBuf>) -> Result<Settings, String> {
    let path = config
        .clone()
        .unwrap_or_else(constants::config_path);
    Settings::load(&path)
}

fn init_logger(settings: &Settings) {
    logger::init(&settings.log_dir, &settings.log_level, true);
}

fn spawn_daemon(config: &Option<PathBuf>) -> Result<u32, String> {
    let mut args: Vec<String> = vec!["--daemon".to_string()];
    if let Some(config) = config {
        args.push("--config".to_string());
        args.push(config.to_string_lossy().to_string());
    }
    let pid = util::spawn_process(
        constants::app_path(),
        &args,
        Some(constants::wd_dir()),
        false,
        true,
        true,
    )
    .map_err(|err| format!("启动后台进程失败: {err}"))?;
    match util::wait_pid_file(&constants::pid_file(), Duration::from_secs(10)) {
        Some(watcher_pid) => Ok(watcher_pid),
        None => Err(format!("后台进程已创建（PID {pid}），但未检测到 watchdog.pid")),
    }
}

fn cmd_start(config: &Option<PathBuf>) -> Result<(), String> {
    if let Some(pid) = util::watcher_running_pid() {
        outln!("看门狗已经在运行（PID {pid}），无需重复启动");
        return Ok(());
    }
    let pid = spawn_daemon(config)?;
    outln!("看门狗已在后台启动（PID {pid}）");
    outln!("  日志目录: {}", constants::wd_dir().join("logs").display());
    outln!("  查看状态: --check    停止: --stop");
    Ok(())
}

fn cmd_ensure(config: &Option<PathBuf>) {
    if util::watcher_running_pid().is_some() {
        return;
    }
    match spawn_daemon(config) {
        Ok(pid) => outln!("监督者: 看门狗未运行，已拉起（PID {pid}）"),
        Err(err) => outln!("监督者: 拉起失败: {err}"),
    }
}

fn cmd_stop() -> Result<(), String> {
    let Some(pid) = util::watcher_running_pid() else {
        for path in [
            constants::pid_file(),
            constants::flag_stop(),
            constants::flag_full_restart(),
            constants::flag_alas_restart(),
        ] {
            let _ = fs::remove_file(path);
        }
        outln!("看门狗未在运行");
        return Ok(());
    };
    fs::write(constants::flag_stop(), util::now_text())
        .map_err(|err| format!("写入停止请求失败: {err}"))?;
    outln!("已发送停止请求（PID {pid}），等待看门狗退出...");
    let deadline = std::time::Instant::now() + Duration::from_secs(300);
    while std::time::Instant::now() < deadline {
        if !crate::win32::is_pid_alive(pid) {
            break;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    if crate::win32::is_pid_alive(pid) {
        outln!("看门狗 300 秒内未退出，强制结束（状态可能不完整）");
        util::taskkill_tree(pid);
        std::thread::sleep(Duration::from_secs(1));
    }
    for path in [
        constants::pid_file(),
        constants::flag_stop(),
        constants::flag_full_restart(),
        constants::flag_alas_restart(),
    ] {
        let _ = fs::remove_file(path);
    }
    outln!("看门狗已停止");
    Ok(())
}

fn cmd_check(config: &Option<PathBuf>) -> Result<(), String> {
    let settings = load_settings(config)?;
    init_logger(&settings);
    let watchdog = Watchdog::new(settings)?;
    watchdog.print_status();
    Ok(())
}

fn cmd_restart_now(config: &Option<PathBuf>) -> Result<bool, String> {
    if util::watcher_running_pid().is_some() {
        fs::write(constants::flag_full_restart(), util::now_text())
            .map_err(|err| format!("写入重启请求失败: {err}"))?;
        outln!("检测到看门狗正在运行，已通知它执行完整重启（详见日志目录）");
        return Ok(true);
    }
    let settings = load_settings(config)?;
    init_logger(&settings);
    let mut watchdog = Watchdog::new(settings)?;
    Ok(watchdog.run_full_cycle("命令行手动请求"))
}

fn cmd_restart_alas_now(config: &Option<PathBuf>) -> Result<bool, String> {
    if util::watcher_running_pid().is_some() {
        fs::write(constants::flag_alas_restart(), util::now_text())
            .map_err(|err| format!("写入重启请求失败: {err}"))?;
        outln!("检测到看门狗正在运行，已通知它重启 Alas（不重启模拟器）");
        return Ok(true);
    }
    let settings = load_settings(config)?;
    init_logger(&settings);
    let mut watchdog = Watchdog::new(settings)?;
    let emulator_ok = watchdog
        .emulator
        .info(None)
        .map(|info| crate::emulator::Emulator::is_running(&info))
        .unwrap_or(false)
        && watchdog.emulator.adb_state() == "device";
    if !emulator_ok {
        outln!("模拟器或 ADB 当前异常，改为执行完整重启");
        return Ok(watchdog.run_full_cycle("命令行手动请求（模拟器/ADB 异常）"));
    }
    Ok(watchdog.restart_alas_only("命令行手动请求"))
}

fn cmd_test_emulator(config: &Option<PathBuf>, index: u32, keep_running: bool) -> Result<bool, String> {
    if let Some(pid) = util::watcher_running_pid() {
        return Err(format!("看门狗正在运行（PID {pid}），请先执行 --stop 再测试"));
    }
    let settings = load_settings(config)?;
    init_logger(&settings);
    let layout = crate::layout::detect_layout(&settings)?;
    let alas = crate::alas::Alas::new(settings.clone(), layout.clone());
    if index == layout.instance && alas.electron_process_count() > 0 {
        return Err(format!(
            "拒绝测试：实例 {index} 正是 Alas 正在使用的模拟器，请先关闭 Alas 或换实例编号"
        ));
    }
    let mut emulator = crate::emulator::Emulator::new(&settings, &layout);
    emulator.instance = index;
    emulator.serial = format!("127.0.0.1:{}", 16384 + index * 32);
    settings.kill_adb_server;
    outln!("{}", "=".repeat(72));
    outln!("模拟器控制实测：实例 {index}（不会动 Alas）");
    outln!("{}", "=".repeat(72));
    let result = (|| -> Result<(), String> {
        match emulator.info(None) {
            Ok(info) if crate::emulator::Emulator::is_running(&info) => {
                outln!("实例当前已在运行，跳过启动步骤");
            }
            _ => {
                outln!("启动实例 ...");
                let info = emulator.launch()?;
                outln!(
                    "启动完成: name={} pid={} adb_port={} state={}",
                    info.get("name").and_then(|value| value.as_str()).unwrap_or("?"),
                    info.get("pid").and_then(|value| value.as_u64()).unwrap_or(0),
                    info.get("adb_port").and_then(|value| value.as_u64()).unwrap_or(0),
                    info.get("player_state").and_then(|value| value.as_str()).unwrap_or("?")
                );
            }
        }
        emulator.wait_adb_ready()?;
        outln!("ADB 正常: {}", emulator.serial);
        outln!("adb state = {}, sys.boot_completed = {}", emulator.adb_state(), {
            let boot = emulator.adb_shell(&["getprop", "sys.boot_completed"], Duration::from_secs(15));
            boot.stdout.trim().to_string()
        });
        Ok(())
    })();
    if !keep_running {
        outln!("关闭测试实例 ...");
        let shutdown_result = emulator.shutdown();
        match shutdown_result {
            Ok(()) => outln!("测试实例已关闭"),
            Err(err) => outln!("关闭测试实例失败: {err}"),
        }
    }
    outln!("{}", "=".repeat(72));
    match result {
        Ok(()) => {
            outln!("测试结果: 通过");
            Ok(true)
        }
        Err(err) => {
            outln!("测试结果: 失败（{err}）");
            Ok(false)
        }
    }
}

fn attach_console_if_needed() {
    if std::env::var("MUW_CMD_LOG").is_ok() {
        return;
    }
    if !crate::win32::console_available() && std::env::args().count() > 1 {
        crate::win32::attach_parent_console();
    }
}

/// 程序入口（返回进程退出码）。
pub fn run() -> i32 {
    attach_console_if_needed();
    let cli = Cli::parse();

    // 不依赖 config.yml 的命令
    if cli.install_autostart {
        return match crate::integrations::install_autostart() {
            Ok(()) => {
                outln!("已安装开机自启（当前用户 Startup 快捷方式）");
                0
            }
            Err(err) => {
                outln!("{err}");
                1
            }
        };
    }
    if cli.uninstall_autostart {
        return match crate::integrations::uninstall_autostart() {
            Ok(()) => {
                outln!("已移除开机自启");
                0
            }
            Err(err) => {
                outln!("{err}");
                1
            }
        };
    }
    if cli.install_supervisor {
        return match crate::integrations::install_supervisor() {
            Ok(()) => {
                outln!(
                    "已安装监督者计划任务「{}」（每 5 分钟执行 --ensure）",
                    constants::SUPERVISOR_TASK_NAME
                );
                0
            }
            Err(err) => {
                outln!("{err}");
                1
            }
        };
    }
    if cli.uninstall_supervisor {
        return match crate::integrations::uninstall_supervisor() {
            Ok(()) => {
                outln!("已移除监督者计划任务");
                0
            }
            Err(err) => {
                outln!("{err}");
                1
            }
        };
    }
    if cli.ensure {
        cmd_ensure(&cli.config);
        return 0;
    }
    if cli.stop {
        return match cmd_stop() {
            Ok(()) => 0,
            Err(err) => {
                outln!("{err}");
                1
            }
        };
    }
    if cli.start {
        return match cmd_start(&cli.config) {
            Ok(()) => 0,
            Err(err) => {
                outln!("{err}");
                1
            }
        };
    }

    if cli.check {
        return match cmd_check(&cli.config) {
            Ok(()) => 0,
            Err(err) => {
                outln!("检查失败: {err}");
                1
            }
        };
    }
    if cli.restart_now {
        return match cmd_restart_now(&cli.config) {
            Ok(true) => 0,
            Ok(false) => 2,
            Err(err) => {
                outln!("{err}");
                2
            }
        };
    }
    if cli.restart_alas_now {
        return match cmd_restart_alas_now(&cli.config) {
            Ok(true) => 0,
            Ok(false) => 2,
            Err(err) => {
                outln!("{err}");
                2
            }
        };
    }
    if let Some(index) = cli.test_emulator {
        return match cmd_test_emulator(&cli.config, index, cli.keep_running) {
            Ok(true) => 0,
            Ok(false) => 2,
            Err(err) => {
                outln!("{err}");
                2
            }
        };
    }

    // 需要 config.yml 的模式
    let settings = match load_settings(&cli.config) {
        Ok(settings) => settings,
        Err(err) => {
            outln!("错误: {err}");
            return 2;
        }
    };
    if settings.created {
        outln!("已生成默认配置文件: {}", settings.path.display());
    }
    if cli.cli || cli.daemon {
        init_logger(&settings);
        return match watchdog::run_daemon(settings) {
            Ok(()) => 0,
            Err(err) => {
                log::error!("看门狗异常退出: {err}");
                1
            }
        };
    }

    // 默认 / --gui：GUI 控制台
    init_logger(&settings);
    match crate::gui::run(settings) {
        Ok(()) => 0,
        Err(err) => {
            outln!("{err}");
            1
        }
    }
}

#[allow(dead_code)]
fn _unused() {
    let _ = Cli::parse_from(["MuMuAlasWatchdog"]);
}
