"""环境布局探测：Alas 根目录、MuMuManager、实例编号、ADB、配置名、WebUI 端口。"""

from __future__ import annotations

import glob
import re
import shutil
from dataclasses import dataclass, field
from pathlib import Path
from typing import Dict, List, Optional

from .config import Settings, read_alas_config, read_deploy_config, resolve_relative
from .win32api import list_processes, proc_exe


class LayoutError(RuntimeError):
    pass


@dataclass
class Layout:
    alas_root: Path
    mumu_manager: Path
    adb_exe: Path
    electron_exe: Path
    launcher_exe: Path
    toolkit_python: Path
    alas_log_dir: Path
    webui_port: int
    serial: str
    instance: int
    instance_name: Optional[str]
    configs: List[str]
    launch_mode: str
    deploy: Dict[str, str] = field(default_factory=dict)
    alas_cfg: Dict = field(default_factory=dict)
    serial_config: str = ""
    emulator_name: str = ""

    @property
    def mumu_root(self) -> Path:
        return self.mumu_manager.parent

    @property
    def launch_exe(self) -> Path:
        return self.launcher_exe if self.launch_mode == "launcher" else self.electron_exe


def _mumu_query_all(manager: Path, timeout: float = 25) -> Dict:
    """布局探测阶段查询 MuMu 实例信息（不依赖完整客户端）。"""
    from .util import run_cmd

    try:
        completed = run_cmd([manager, "info", "-v", "all"], timeout=timeout)
    except Exception:
        return {}
    text = (completed.out or "").strip()
    if not text.startswith("{"):
        return {}
    import json

    try:
        data = json.loads(text)
    except Exception:
        return {}
    return data if isinstance(data, dict) else {}


def _detect_alas_root(configured: str) -> Path:
    candidates: List[Path] = []
    if configured and configured.lower() != "auto":
        candidate = Path(configured)
        if not candidate.is_absolute():
            candidate = Path(__file__).resolve().parent.parent / candidate
        candidates.append(candidate)
    project_root = Path(__file__).resolve().parent.parent
    candidates.extend(
        [
            project_root.parent / "AzurLaneAutoScript",
            project_root / "AzurLaneAutoScript",
            project_root.parent.parent / "AzurLaneAutoScript",
        ]
    )
    # 从运行中的 Alas Electron 进程反推：<root>/toolkit/webapp/alas.exe
    for process in list_processes():
        if process.name.lower() != "alas.exe":
            continue
        executable = proc_exe(process.pid)
        if not executable:
            continue
        exe_path = Path(executable)
        parts = [part.lower() for part in exe_path.parts]
        if "toolkit" not in parts or "webapp" not in parts:
            continue
        try:
            candidates.append(exe_path.parents[2])
        except IndexError:
            continue
    for candidate in candidates:
        try:
            resolved = candidate.resolve()
            if (resolved / "alas.py").exists() and (resolved / "gui.py").exists():
                return resolved
        except OSError:
            continue
    raise LayoutError(
        "找不到 Alas 目录：请在 config.yml 的 paths.alas_root 填写 AzurLaneAutoScript 的完整路径"
    )


def _detect_mumu_manager(configured: str, emulator_path: str) -> Path:
    candidates: List[Path] = []
    if configured and configured.lower() != "auto":
        candidate = Path(configured)
        if not candidate.is_absolute():
            candidate = Path(__file__).resolve().parent.parent / candidate
        candidates.append(candidate)
    if emulator_path:
        candidates.append(Path(emulator_path).parent / "MuMuManager.exe")
        candidates.append(Path(emulator_path).parent.parent / "shell" / "MuMuManager.exe")
    for process in list_processes():
        if process.name.lower() not in ("mumunxmain.exe", "mumuplayer.exe"):
            continue
        executable = proc_exe(process.pid)
        if executable:
            candidates.append(Path(executable).parent / "MuMuManager.exe")
            candidates.append(Path(executable).parent / "shell" / "MuMuManager.exe")
    for pattern in (
        r"C:\Program Files\Netease\MuMuPlayer*\nx_main\MuMuManager.exe",
        r"C:\Program Files\Netease\MuMuPlayer*\shell\MuMuManager.exe",
        r"D:\Program Files\Netease\MuMuPlayer*\nx_main\MuMuManager.exe",
        r"E:\Program Files\Netease\MuMuPlayer*\nx_main\MuMuManager.exe",
    ):
        candidates.extend(Path(path) for path in glob.glob(pattern))
    for candidate in candidates:
        try:
            if candidate.exists():
                return candidate.resolve()
        except OSError:
            continue
    raise LayoutError("找不到 MuMuManager.exe：请在 config.yml 的 paths.mumu_manager 填写完整路径")


def _detect_instance(
    configured: str,
    serial_config: str,
    emulator_name: str,
    manager: Path,
) -> tuple[int, Optional[str]]:
    index: Optional[int] = None
    if configured and str(configured).lower() != "auto":
        try:
            index = int(configured)
        except (TypeError, ValueError):
            index = None

    serial_port: Optional[int] = None
    if serial_config and ":" in serial_config:
        try:
            serial_port = int(serial_config.rsplit(":", 1)[1])
        except ValueError:
            serial_port = None

    all_info = _mumu_query_all(manager)
    if index is None and serial_port is not None:
        for key, item in all_info.items():
            if isinstance(item, dict) and item.get("adb_port") == serial_port:
                try:
                    index = int(key)
                except ValueError:
                    index = None
                break
    if index is None:
        name_match = re.search(r"-(\d+)$", emulator_name or "")
        if name_match:
            index = int(name_match.group(1))
    if index is None and all_info and emulator_name:
        for key, item in all_info.items():
            if isinstance(item, dict) and item.get("name") == emulator_name:
                try:
                    index = int(key)
                except ValueError:
                    index = None
                break
    if index is None:
        index = 0

    instance_name: Optional[str] = None
    item = all_info.get(str(index)) if all_info else None
    if isinstance(item, dict):
        instance_name = item.get("name")
    if not instance_name:
        instance_name = emulator_name or None
    return index, instance_name


def _detect_adb(configured: str, alas_root: Path, deploy: Dict[str, str]) -> Path:
    candidates: List[Path] = []
    if configured and configured.lower() != "auto":
        candidate = Path(configured)
        if not candidate.is_absolute():
            candidate = Path(__file__).resolve().parent.parent / candidate
        candidates.append(candidate)
    else:
        deployed = str(deploy.get("AdbExecutable") or "").strip()
        if deployed and deployed.lower() != "null":
            candidates.append(resolve_relative(alas_root, deployed))
        candidates.append(
            alas_root / "toolkit" / "Lib" / "site-packages" / "adbutils" / "binaries" / "adb.exe"
        )
        system_adb = shutil.which("adb")
        if system_adb:
            candidates.append(Path(system_adb))
    for candidate in candidates:
        try:
            if candidate.exists():
                return candidate.resolve()
        except OSError:
            continue
    raise LayoutError("找不到 adb.exe：请在 config.yml 的 paths.adb_exe 填写完整路径")


def _detect_config_names(configured, alas_root: Path) -> List[str]:
    if isinstance(configured, (list, tuple)):
        names = [str(item).strip() for item in configured if str(item).strip()]
        if names:
            return names
    elif configured and str(configured).lower() != "auto":
        names = [item.strip() for item in re.split(r"[,;\s]+", str(configured)) if item.strip()]
        if names:
            return names
    config_dir = alas_root / "config"
    names = []
    for path in sorted(config_dir.glob("*.json")):
        if path.stem.startswith("template") or path.stem == "deploy":
            continue
        names.append(path.stem)
    if not names:
        raise LayoutError(f"在 {config_dir} 里找不到 Alas 配置文件（<配置名>.json）")
    return names


def detect_layout(settings: Settings) -> Layout:
    """根据配置 + 运行环境探测出完整布局。"""
    alas_root = _detect_alas_root(settings.alas_root)
    deploy = read_deploy_config(alas_root)
    alas_cfg = read_alas_config(alas_root)
    alas_group = alas_cfg.get("Alas") if isinstance(alas_cfg.get("Alas"), dict) else {}
    emulator = alas_group.get("Emulator") if isinstance(alas_group.get("Emulator"), dict) else {}
    emulator_info = alas_group.get("EmulatorInfo") if isinstance(alas_group.get("EmulatorInfo"), dict) else {}
    serial_config = str(emulator.get("Serial") or "").strip()
    emulator_name = str(emulator_info.get("name") or "").strip()
    emulator_path = str(emulator_info.get("path") or "").strip()

    mumu_manager = _detect_mumu_manager(settings.mumu_manager, emulator_path)
    instance, instance_name = _detect_instance(
        str(settings.mumu_instance), serial_config, emulator_name, mumu_manager
    )
    serial = serial_config or f"127.0.0.1:{16384 + 32 * instance}"
    adb_exe = _detect_adb(settings.adb_exe, alas_root, deploy)

    port_text = str(deploy.get("WebuiPort") or "").strip()
    try:
        webui_port = int(float(port_text)) if port_text and port_text.lower() != "null" else 22267
    except ValueError:
        webui_port = 22267

    configs = _detect_config_names(settings.config_names, alas_root)

    launch_mode = settings.launch_mode
    electron_exe = alas_root / "toolkit" / "webapp" / "alas.exe"
    launcher_exe = alas_root / "Alas.exe"
    if launch_mode == "launcher" and not launcher_exe.exists():
        launch_mode = "electron"
    if launch_mode == "electron" and not electron_exe.exists():
        if launcher_exe.exists():
            launch_mode = "launcher"
        else:
            raise LayoutError(f"找不到 Alas 启动文件: {electron_exe}")
    if not (alas_root / "gui.py").exists():
        raise LayoutError(f"Alas 目录不完整（没有 gui.py）: {alas_root}")

    return Layout(
        alas_root=alas_root,
        mumu_manager=mumu_manager,
        adb_exe=adb_exe,
        electron_exe=electron_exe,
        launcher_exe=launcher_exe,
        toolkit_python=alas_root / "toolkit" / "python.exe",
        alas_log_dir=alas_root / "log",
        webui_port=webui_port,
        serial=serial,
        instance=instance,
        instance_name=instance_name,
        configs=configs,
        launch_mode=launch_mode,
        deploy=deploy,
        alas_cfg=alas_cfg,
        serial_config=serial_config,
        emulator_name=emulator_name,
    )


__all__ = ["Layout", "LayoutError", "detect_layout"]
