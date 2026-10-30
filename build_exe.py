"""使用 PyInstaller 打包单文件窗口化 exe。

用法:
    .\\.venv\\Scripts\\python.exe build_exe.py
输出:
    dist/MuMuAlasWatchdog.exe
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent


def main() -> int:
    command = [
        sys.executable,
        "-m",
        "PyInstaller",
        "--noconfirm",
        "--clean",
        "--onefile",
        "--windowed",
        "--name",
        "MuMuAlasWatchdog",
        "--distpath",
        str(ROOT / "dist"),
        "--workpath",
        str(ROOT / "build"),
        "--specpath",
        str(ROOT / "build"),
        str(ROOT / "main.py"),
    ]
    print("运行:", " ".join(command))
    code = subprocess.call(command)
    if code != 0:
        return code
    built = ROOT / "dist" / "MuMuAlasWatchdog.exe"
    target = ROOT / "MuMuAlasWatchdog.exe"
    if built.exists():
        shutil.copy2(built, target)
        print(f"已生成: {target}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
