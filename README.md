# MuMu + Alas 自动重启看门狗（Rust + egui）

用 Rust + egui 编写的 Windows 无人值守守护工具，面向 **MuMu 模拟器 + AzurLaneAutoScript（Alas）**：
按计划完成「关 Alas → 重启 MuMu 实例 → 等 ADB 恢复 → 启动 Alas → 自动开始游戏脚本」，
运行期间持续巡检、异常自愈。**不需要 OCR，也不修改 Alas 任何代码。**

> Rust 版为当前正式版本（v0.1.0）。原 Python 版归档在 `F:\自动化脚本-python-legacy`，
> GitHub `python-legacy` 分支保留其历史。

![控制台](docs/gui-rust.png)

## 特性

- **计划重启**：每天固定时刻（如 04:00、16:00）或按间隔小时（默认 6 小时）；
- **启动自愈**：启动时模拟器或 Alas 未运行会自动拉起；
- **巡检自愈**：ADB 掉线、模拟器退出、Alas 进程/WebUI 丢失、任务长时间无日志、模拟器内存超限；
- **托盘常驻**：后台守护无任务栏窗口，只有系统托盘图标和右键菜单，不是“完全隐身”；
- **egui 控制台**：运行概览 / 配置 / 运行日志 / 外观 四个页面；
- **GUI 内配置**：直接编辑 config.yml，保存后自动请求后台热重载；
- **外观定制**：背景与控件透明度、内置/自定义背景、统一主题；
- **长期无人值守**：开机自启 + 计划任务监督者（每 2 分钟 `--ensure`）+ 自身资源阈值自重启；
- **可观测**：按天滚动日志、status.json、自身 CPU/内存/句柄指标、panic 落盘 `crash.log`；
- **单实例**：重复打开 GUI 会唤起已有窗口；后台只允许一个实例；
- **低开销**：守护进程空闲约 0.1% 单核 / 15 MB 内存。

## 工作原理

完整重启流程：

1. 结束 Alas（Electron / toolkit python）进程；
2. 用 MuMuManager 关闭指定 MuMu 实例；
3. 启动实例并等待启动完成；
4. 重启 adb server，连接并等待 `sys.boot_completed`；
5. 写入 `config/reloadalas`，用 Win32 `CreateProcessW` 启动 Alas；
6. 等待 WebUI 就绪，并等待每个 Alas 配置出现 `Start scheduler loop`。

巡检（默认每 60 秒）：

- ADB 状态与响应时间；
- 模拟器进程存活、提交内存（可选上限）；
- Alas Electron 主进程数、WebUI 可访问性；
- 当前任务的最后日志时间（疑似卡死可触发完整重启）；
- 失败重试与冷却：重启失败按指数退避重试，避免连续失败刷屏。

> MuMuManager 单次 `info` 查询实测消耗 100~220ms CPU，因此只在状态采集周期（默认 300 秒）
> 或进程丢失时查询；平时的存活检查使用缓存的 PID，避免每分钟启动 MuMuManager。

## 系统托盘与窗口行为

守护进程创建一个不可见消息窗口（不进入任务栏），通过 `Shell_NotifyIcon` 注册托盘图标，
悬浮提示显示下次重启时间。

单击托盘图标：

- 控制台未运行：启动控制台；
- 控制台已打开：恢复并前置已有窗口。

右键菜单：

- 打开控制台
- 立即完整重启 / 只重启 Alas
- 打开日志目录
- 配置（控制台）
- 退出看门狗

关闭控制台窗口（X / Alt+F4 / “退出界面”）**只退出 GUI**，后台看门狗与托盘图标继续运行；
再次单击托盘图标即可打开控制台。只有后台看门狗注册托盘图标（GUI 不注册第二份），
因此任务栏通知区域始终只有一个程序图标。

> 为什么不做“隐藏窗口到托盘”：实测 eframe/winit 在窗口隐藏后事件循环仍全速空转，
> 隐藏窗口可占用 **90%+ 单核**。因此采用“关闭控制台 = 退出 GUI，后台常驻托盘”的方案，
> 既符合“关闭回托盘”的直觉，又避免隐藏窗口空转。

要完全退出守护进程：托盘右键“退出看门狗”，或执行 `MuMuAlasWatchdog.exe --stop`。

## 构建

要求：Windows 10/11 x64、Rust 1.88+（MSVC 工具链）。

```powershell
cd F:\自动化脚本
cargo build --release
# 产物: target\release\MuMuAlasWatchdog.exe（约 9.2 MB）
```

部署时只需要：

- `MuMuAlasWatchdog.exe`
- `config.example.yml`（首次运行生成 config.yml，之后读取 config.yml）
- 可选 `icon.ico`（托盘图标优先使用 exe 内嵌资源，文件仅作兜底）

托盘/窗口图标和背景图片通过 `build.rs` 在构建期嵌入，`assets/` 不需要随程序分发。
`cargo build`（debug）会生成带控制台的版本，便于查看 CLI 日志。

## 使用

```powershell
# GUI（默认，双击 exe 等价）
MuMuAlasWatchdog.exe
MuMuAlasWatchdog.exe --gui

# 后台守护
MuMuAlasWatchdog.exe --start       # 无窗口后台启动，托盘出现图标
MuMuAlasWatchdog.exe --stop        # 有序停止，会等当前重启操作结束
MuMuAlasWatchdog.exe --cli         # 前台 CLI 守护（控制台日志，调试用）

# 状态与手动操作
MuMuAlasWatchdog.exe --check
MuMuAlasWatchdog.exe --restart-now
MuMuAlasWatchdog.exe --restart-alas-now
MuMuAlasWatchdog.exe --test-emulator 0                 # 实测指定实例启停，不影响 Alas
MuMuAlasWatchdog.exe --test-emulator 0 --keep-running  # 测试后保留实例运行

# 长期无人值守
MuMuAlasWatchdog.exe --install-autostart
MuMuAlasWatchdog.exe --uninstall-autostart
MuMuAlasWatchdog.exe --install-supervisor     # 计划任务：每 2 分钟 --ensure
MuMuAlasWatchdog.exe --uninstall-supervisor

# 指定配置文件
MuMuAlasWatchdog.exe --config D:\path\config.yml
```

- 看门狗运行时，`--restart-now` / `--restart-alas-now` 会写控制标记，由后台在主循环安全点执行；
  未运行时则由当前进程直接执行。
- `--ensure` 是监督者内部命令：看门狗未运行则拉起，已运行则直接退出。
- `--test-emulator INDEX` 要求看门狗已停止，避免抢占正在使用的实例。

## 配置

`config.yml` 与 exe 同目录；首次运行从 `config.example.yml` 复制。也可在 GUI「配置」页编辑保存。

| 分组 | 说明 |
| --- | --- |
| `paths` | Alas 根目录、MuMuManager.exe、实例编号、adb.exe、启动方式（`auto` = 自动识别） |
| `alas` | 需要恢复的 Alas 配置名（对应 `config/<名字>.json`） |
| `schedule` | 固定重启时刻 / 间隔小时、启动立即重启、启动自愈、失败重试、首次宽限 |
| `health` | 巡检间隔、ADB 失败阈值、模拟器内存上限、模拟器/Alas 退出自愈、任务卡死判定与冷却 |
| `timeouts` | 关闭/启动模拟器、ADB 就绪、Alas 启动、调度器启动超时 |
| `process` | adb server 清理、`reloadalas` 自动开始、结束 Alas 等待时间 |
| `monitor` | 状态日志间隔、status.json、自身内存/句柄/运行时长自重启阈值 |
| `logging` | 日志目录、保留天数、日志级别 |
| `advanced` | MuMuManager 命令超时 |

- 保存配置时保留未知键，并把原文件备份为 `config.yml.bak`；
- 保存后自动请求后台热重载：后台在主循环安全点校验并应用配置，失败则保留旧配置并显示原因；
- 外部修改配置后，可在 GUI 点击“热重载已保存配置”；
- 外观设置独立保存在 `gui-appearance.json`，不触发后台配置热重载；
- 自我监控阈值配合监督者任务：超限时记录日志并有序退出，2 分钟内由 `--ensure` 重新拉起。

## 日志、状态与可观测性

- `logs/watchdog_YYYYMMDD.log`：按天滚动，保留 14 天；
- `status.json`：模拟器 / ADB / Alas 状态、看门狗自身 WS/Private/Handles/CPU、下次重启、上次重启结果；
- 每 `monitor.status_log_seconds` 输出一条状态日志，例如：

  ```text
  [状态] 看门狗自身: WS 15.1 MB / Handles 174 / CPU 0.08% | 计划: 2026-10-31 06:21:04
  ```

- `crash.log`：panic 时写入原因与调用栈（窗口化进程没有控制台，便于事后定位）；
- 托盘悬浮提示同步显示下次重启时间。

### 性能参考（本机实测）

| 项目 | 实测 |
| --- | ---: |
| 守护进程空闲 CPU | 0.08% 单核（≈0.005% 整机 16 线程） |
| 守护进程常驻内存 | WS ≈ 15 MB / Handles ≈ 174 |
| GUI 可见时 CPU | ≈ 0.1% 单核（2 秒重绘一次） |
| MuMuManager 探测频率 | 常态 60 秒内 0 次，仅 5 分钟状态采集或进程丢失时 |
| 完整重启耗时 | ≈ 40 秒 |

## GUI

- **运行概览**：状态卡、下次重启、看门狗 CPU/内存/句柄、启动/停止/重启按钮、开机自启与监督者安装；
- **配置**：路径选择、计划/健康/超时/监控等结构化编辑，保存并热重载；
- **运行日志**：自动刷新最近日志，可打开日志目录；
- **外观**：背景与控件透明度拖动实时预览、内置/自定义背景、恢复默认、保存外观。

界面统一使用 #0096FA 主题和微软雅黑（mmap 共享系统字体，避免重复解析 TTC）；
背景按窗口等比铺满裁切，支持 PNG/JPEG/WebP（≤32 MB、边长 ≤8192），文件缺失时回退内置背景。
外观设置保存到 `gui-appearance.json`，下次启动自动恢复。

## 目录结构

```text
F:\自动化脚本\
├─ src\
│  ├─ main.rs / lib.rs          # 入口与模块导出
│  ├─ cli.rs                    # CLI 子命令（GUI / 后台 / 维护命令）
│  ├─ watchdog.rs               # 主编排：重启、巡检、调度、自监控、单实例锁
│  ├─ emulator.rs / alas.rs     # MuMuManager 与 Alas 控制
│  ├─ layout.rs / logwatch.rs   # 环境探测与日志增量读取
│  ├─ schedule.rs / status.rs   # 计划计算与状态采集
│  ├─ config.rs                 # config.yml 读取
│  ├─ config_editor.rs + config_editor\  # 配置草稿、字段、中文标签
│  ├─ gui.rs                    # 原生窗口入口
│  ├─ gui\app.rs + app\         # 页面、状态、关闭/窗口生命周期
│  ├─ gui\theme\ + background.rs + fonts.rs + worker.rs
│  ├─ tray.rs                   # 系统托盘（隐藏窗口 + Shell_NotifyIcon）
│  ├─ instance.rs / reload.rs   # 单实例互斥与配置热重载
│  ├─ appearance.rs             # 外观配置模型与读写
│  ├─ integrations.rs           # 开机自启、计划任务监督者
│  ├─ logger.rs / output.rs     # 按天日志、自身指标、输出
│  ├─ win32.rs / util.rs        # Win32 与通用工具
│  └─ constants.rs
├─ assets\                      # 构建期图标/背景源图（运行时不需）
├─ build.rs / app.rc            # 资源嵌入
├─ docs\gui-rust.png            # 控制台截图
├─ tests\core.rs                # 计划/工具集成测试
├─ config.example.yml           # 配置示例
├─ icon.ico                     # 托盘图标兜底
└─ MuMuAlasWatchdog.exe         # 正式入口
```

## 与 Python 版的关系

- Rust 版：`F:\自动化脚本`（当前正式运行版本）
- Python 版：`F:\自动化脚本-python-legacy`（归档保留，可回退）
- GitHub：`main` = Rust 版；`python-legacy` 分支 = 原 Python 版历史

回退方法：先 `MuMuAlasWatchdog.exe --stop` 停 Rust 看门狗，再到
`F:\自动化脚本-python-legacy` 执行 `python -m mumu_alas_watchdog --start` 并重新安装自启。

## 验证记录（2026-10-31）

- `cargo test --release`：**20 项单元测试 + 7 项集成测试**全部通过；
- `cargo build --release`：单文件窗口化 exe ≈ 9.2 MB；
- 完整重启实测：关 Alas → 关 MuMu → 启 MuMu → ADB 恢复 → Win32 启动 Alas → 调度器自动开始，
  用时约 40 秒，Alas 日志出现 `Start scheduler loop`；
- 托盘：`Shell_NotifyIconGetRect` 成功，图标位于通知区域；隐藏消息窗口 `IsWindowVisible=False`；
- 关闭控制台 0.5 秒退出，后台守护与托盘图标保留；单击托盘可在数秒内重新打开控制台；
- 守护进程空闲 CPU 0.08% 单核；MuMuManager 探测在 60 秒内 0 次；
- 监督者计划任务每 2 分钟执行 `--ensure`，Last Result 0；开机自启指向正式 exe。

## 注意

1. 定时重启会中断当前 Alas 任务；手动玩游戏前请先 `--stop`（或托盘“退出看门狗”）；
2. 看门狗、模拟器和 Alas 都依赖当前用户的交互桌面会话，**不要注册为 Windows 服务**；
3. 开机自启和监督者任务只在当前用户登录后运行；
4. 首次在新机器运行前，请确认 `config.yml` 的 `paths.alas_root` / `paths.mumu_manager`；
5. Windows 可能把新托盘图标收进“隐藏的图标”溢出区，可在任务栏设置中固定显示；
6. 旧版本进程仍在运行时，先正常退出一次再启动新版，才能获得配置热重载等新能力。
