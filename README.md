# MuMu + Alas 自动重启看门狗（Rust + egui 重写）

用 Rust + egui 重写 [Python 版](../自动化脚本) 的无人值守守护工具：
按计划完成「关 Alas → 重启 MuMu 实例 → 等 ADB 恢复 → 启动 Alas → 自动开始游戏脚本」，
运行期间持续巡检、异常自愈，**不需要 OCR，也不修改 Alas 任何代码**。

> 状态：核心功能、GUI、CLI、开机自启/监督者任务、状态与自我监控均已实现并完成本机实测，
> 当前已接管正式运行（Rust 守护进程 + 监督者计划任务 + 开机自启）。

![Rust GUI 控制台](docs/gui-rust.png)

## 为什么用 Rust + egui

- 无常驻解释器运行时，内存/句柄行为可控，可长时间无人值守；
- 单文件原生 exe，无 PyInstaller 解包与残留进程问题；
- Win32 API 直接调用（`windows-sys`），启动进程时显式清洗环境变量；
- egui 原生 GUI + 后台无窗口模式，一套二进制同时支持 GUI / CLI；
- 目标：长期无人值守 + 可观测（自身内存、句柄、运行时长阈值自重启）。

## 构建

```powershell
cd F:\自动化脚本-rust
cargo build --release
# 产物: target\release\MuMuAlasWatchdog.exe
```

首次编译依赖较多（egui/wgpu 等），之后增量很快。

## 使用

```powershell
# GUI（默认）：双击 exe，或
.\target\release\MuMuAlasWatchdog.exe

# CLI
.\target\release\MuMuAlasWatchdog.exe --check
.\target\release\MuMuAlasWatchdog.exe --start
.\target\release\MuMuAlasWatchdog.exe --stop
.\target\release\MuMuAlasWatchdog.exe --restart-now
.\target\release\MuMuAlasWatchdog.exe --restart-alas-now
.\target\release\MuMuAlasWatchdog.exe --cli          # 前台 CLI 守护（调试）

# 长期无人值守（推荐同时安装两项）
.\target\release\MuMuAlasWatchdog.exe --install-autostart    # 登录时后台启动
.\target\release\MuMuAlasWatchdog.exe --install-supervisor   # 每 5 分钟 --ensure 兜底
```

`--install-supervisor` 创建名为 `MuMuAlasWatchdog-Supervisor` 的计划任务：
每 5 分钟执行一次 `MuMuAlasWatchdog.exe --ensure`；若看门狗进程不存在则拉起。
配合 `monitor.self_restart_*` 阈值，看门狗在自身资源超限时会记录日志并有序退出，由监督者重新拉起。

## 配置

首次运行从 [config.example.yml](config.example.yml) 复制为 `config.yml`（已被 .gitignore 忽略）。
主要配置与 Python 版一致，额外增加自我监控：

```yaml
monitor:
  status_log_seconds: 300
  write_status_json: true
  self_restart_memory_mb: 200    # 自身工作集超过 200MB 时有序自重启（需监督者）
  self_restart_handles: 1000     # 句柄数上限
  self_restart_after_hours: 24   # 连续运行多久后主动重启一次
```

## 运行状态与日志

- `logs/watchdog_YYYYMMDD.log`：按天滚动，保留 14 天；
- `status.json`：模拟器 / ADB / Alas / 看门狗自身资源 / 上次重启结果；
- 每 `monitor.status_log_seconds` 输出一条状态日志，包含自身 WS/Private/Handles。

## 目录结构

```text
src/
├─ main.rs / lib.rs        # 入口与模块导出
├─ cli.rs                  # CLI 子命令（GUI/CLI 双模式）
├─ gui.rs                  # egui 控制台
├─ watchdog.rs             # 主编排：重启流程、巡检、调度、自监控、单实例锁
├─ emulator.rs             # MuMuManager CLI 封装、启停、ADB 等待
├─ alas.rs                 # Alas 进程识别/结束、Win32 启动、WebUI 就绪
├─ logwatch.rs             # Alas 日志增量读取、任务状态、调度器等待
├─ schedule.rs             # 重启计划纯函数（可单测）
├─ config.rs               # config.yml 读取
├─ layout.rs               # 环境探测（Alas/MuMu/实例/ADB/端口）
├─ win32.rs                # Win32 API：进程枚举/内存/句柄/控制台
├─ status.rs               # 状态采集与 status.json
├─ integrations.rs         # 开机自启、计划任务监督者
├─ logger.rs               # 按天滚动日志 + 自身指标
└─ util.rs / output.rs     # 命令执行、HTTP、路径、输出
tests/core.rs              # schedule / util 单元测试
```

## 与 Python 版的关系

Python 版源码仍在 `F:\自动化脚本`（保留作回退），但其守护进程已停止；
开机自启快捷方式已改指向 Rust exe，监督者计划任务也由 Rust 版安装。
如需回退：先 `MuMuAlasWatchdog.exe --stop` 停 Rust 看门狗，再到 Python 目录执行
`python -m mumu_alas_watchdog --start` 并重新 `--install-autostart`。

## 本机验证记录

- `cargo test --release`：7 项单元测试全部通过（计划计算、时间格式、路径规则等）；
- `cargo build --release`：单文件窗口化 exe 约 5.4 MB；
- CLI：`--check` 状态读取正确（模拟器/ADB/WebUI/日志任务状态）；
- `--install-supervisor` / `--uninstall-supervisor`：计划任务创建、查询、删除验证通过（Last Result 0）；
- GUI：egui 窗口正常渲染（中文字体、状态行、按钮、日志面板）；
- 完整重启实测：关 Alas → 关 MuMu → 启动 MuMu → ADB 恢复 → Win32 启动 Alas → 调度器自动开始，
  用时约 40 秒，Alas 日志确认出现 `Start scheduler loop`；
- 当前部署：Rust 守护进程运行中（WS ~11.5 MB / Handles ~137 / 2 threads），
  下次计划重启 22:53:33；监督者计划任务每 5 分钟执行 `--ensure`；开机自启指向 Rust exe。

## 注意

1. 定时重启会中断当前 Alas 任务；
2. 手动玩之前先 `--stop`；
3. 开机自启/监督者任务都依赖当前用户登录会话（MuMu/Alas GUI 需要交互桌面）；
4. 首次在新机器运行前，请在 `config.yml` 确认 `paths.alas_root` / `paths.mumu_manager`。
