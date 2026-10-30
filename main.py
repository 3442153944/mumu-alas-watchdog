"""PyInstaller 打包入口（源码运行请使用 python -m mumu_alas_watchdog）。"""

import sys

from mumu_alas_watchdog.cli import main

if __name__ == "__main__":
    sys.exit(main())
