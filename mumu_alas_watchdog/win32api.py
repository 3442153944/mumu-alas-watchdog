"""Windows API 封装（ctypes）：进程枚举、内存、创建时间、CreateProcessW 启动。

所有启动动作都走 Win32 API，不经过 shell；环境变量完全可控。
"""

from __future__ import annotations

import ctypes
import ctypes.wintypes as wt
import os
import subprocess
import sys
from dataclasses import dataclass
from datetime import datetime
from typing import Dict, List, Optional

kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
psapi = ctypes.WinDLL("psapi", use_last_error=True)

TH32CS_SNAPPROCESS = 0x00000002
INVALID_HANDLE_VALUE = ctypes.c_void_p(-1).value
PROCESS_QUERY_LIMITED_INFORMATION = 0x1000
PROCESS_QUERY_INFORMATION = 0x0400
PROCESS_VM_READ = 0x0010
SYNCHRONIZE = 0x00100000
WAIT_TIMEOUT = 0x00000102
MAX_PATH = 260

CREATE_UNICODE_ENVIRONMENT = 0x00000400
STARTF_USESHOWWINDOW = 0x00000001
SW_HIDE = 0
ATTACH_PARENT_PROCESS = 0xFFFFFFFF


class PROCESSENTRY32W(ctypes.Structure):
    _fields_ = [
        ("dwSize", wt.DWORD),
        ("cntUsage", wt.DWORD),
        ("th32ProcessID", wt.DWORD),
        ("th32DefaultHeapID", ctypes.c_size_t),
        ("th32ModuleID", wt.DWORD),
        ("cntThreads", wt.DWORD),
        ("th32ParentProcessID", wt.DWORD),
        ("pcPriClassBase", wt.LONG),
        ("dwFlags", wt.DWORD),
        ("szExeFile", wt.WCHAR * MAX_PATH),
    ]


class FILETIME(ctypes.Structure):
    _fields_ = [("dwLowDateTime", wt.DWORD), ("dwHighDateTime", wt.DWORD)]


class PROCESS_MEMORY_COUNTERS_EX(ctypes.Structure):
    _fields_ = [
        ("cb", wt.DWORD),
        ("PageFaultCount", wt.DWORD),
        ("PeakWorkingSetSize", ctypes.c_size_t),
        ("WorkingSetSize", ctypes.c_size_t),
        ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPagedPoolUsage", ctypes.c_size_t),
        ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
        ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
        ("PagefileUsage", ctypes.c_size_t),
        ("PeakPagefileUsage", ctypes.c_size_t),
        ("PrivateUsage", ctypes.c_size_t),
    ]


class STARTUPINFOW(ctypes.Structure):
    _fields_ = [
        ("cb", wt.DWORD),
        ("lpReserved", wt.LPWSTR),
        ("lpDesktop", wt.LPWSTR),
        ("lpTitle", wt.LPWSTR),
        ("dwX", wt.DWORD),
        ("dwY", wt.DWORD),
        ("dwXSize", wt.DWORD),
        ("dwYSize", wt.DWORD),
        ("dwXCountChars", wt.DWORD),
        ("dwYCountChars", wt.DWORD),
        ("dwFillAttribute", wt.DWORD),
        ("dwFlags", wt.DWORD),
        ("wShowWindow", wt.WORD),
        ("cbReserved2", wt.WORD),
        ("lpReserved2", ctypes.c_void_p),
        ("hStdInput", wt.HANDLE),
        ("hStdOutput", wt.HANDLE),
        ("hStdError", wt.HANDLE),
    ]


class PROCESS_INFORMATION(ctypes.Structure):
    _fields_ = [
        ("hProcess", wt.HANDLE),
        ("hThread", wt.HANDLE),
        ("dwProcessId", wt.DWORD),
        ("dwThreadId", wt.DWORD),
    ]


kernel32.CreateToolhelp32Snapshot.argtypes = [wt.DWORD, wt.DWORD]
kernel32.CreateToolhelp32Snapshot.restype = wt.HANDLE
kernel32.Process32FirstW.argtypes = [wt.HANDLE, ctypes.POINTER(PROCESSENTRY32W)]
kernel32.Process32FirstW.restype = wt.BOOL
kernel32.Process32NextW.argtypes = [wt.HANDLE, ctypes.POINTER(PROCESSENTRY32W)]
kernel32.Process32NextW.restype = wt.BOOL
kernel32.CloseHandle.argtypes = [wt.HANDLE]
kernel32.CloseHandle.restype = wt.BOOL
kernel32.OpenProcess.argtypes = [wt.DWORD, wt.BOOL, wt.DWORD]
kernel32.OpenProcess.restype = wt.HANDLE
kernel32.QueryFullProcessImageNameW.argtypes = [
    wt.HANDLE,
    wt.DWORD,
    wt.LPWSTR,
    ctypes.POINTER(wt.DWORD),
]
kernel32.QueryFullProcessImageNameW.restype = wt.BOOL
kernel32.GetProcessTimes.argtypes = [
    wt.HANDLE,
    ctypes.POINTER(FILETIME),
    ctypes.POINTER(FILETIME),
    ctypes.POINTER(FILETIME),
    ctypes.POINTER(FILETIME),
]
kernel32.GetProcessTimes.restype = wt.BOOL
kernel32.WaitForSingleObject.argtypes = [wt.HANDLE, wt.DWORD]
kernel32.WaitForSingleObject.restype = wt.DWORD
kernel32.AttachConsole.argtypes = [wt.DWORD]
kernel32.AttachConsole.restype = wt.BOOL
kernel32.CreateProcessW.argtypes = [
    wt.LPCWSTR,
    wt.LPWSTR,
    ctypes.c_void_p,
    ctypes.c_void_p,
    wt.BOOL,
    wt.DWORD,
    ctypes.c_void_p,
    wt.LPCWSTR,
    ctypes.POINTER(STARTUPINFOW),
    ctypes.POINTER(PROCESS_INFORMATION),
]
kernel32.CreateProcessW.restype = wt.BOOL
psapi.GetProcessMemoryInfo.argtypes = [
    wt.HANDLE,
    ctypes.POINTER(PROCESS_MEMORY_COUNTERS_EX),
    wt.DWORD,
]
psapi.GetProcessMemoryInfo.restype = wt.BOOL


@dataclass
class ProcessInfo:
    pid: int
    ppid: int
    name: str
    exe: Optional[str] = None


def list_processes() -> List[ProcessInfo]:
    """枚举所有进程（Toolhelp32）。"""
    processes: List[ProcessInfo] = []
    snapshot = kernel32.CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
    if snapshot in (None, INVALID_HANDLE_VALUE):
        return processes
    try:
        entry = PROCESSENTRY32W()
        entry.dwSize = ctypes.sizeof(entry)
        ok = kernel32.Process32FirstW(snapshot, ctypes.byref(entry))
        while ok:
            processes.append(
                ProcessInfo(
                    pid=int(entry.th32ProcessID),
                    ppid=int(entry.th32ParentProcessID),
                    name=entry.szExeFile,
                )
            )
            ok = kernel32.Process32NextW(snapshot, ctypes.byref(entry))
    finally:
        kernel32.CloseHandle(snapshot)
    return processes


def proc_exe(pid: int) -> Optional[str]:
    handle = kernel32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, int(pid))
    if not handle:
        return None
    try:
        buffer = ctypes.create_unicode_buffer(32768)
        size = wt.DWORD(len(buffer))
        if kernel32.QueryFullProcessImageNameW(handle, 0, buffer, ctypes.byref(size)):
            return buffer.value or None
        return None
    finally:
        kernel32.CloseHandle(handle)


def is_pid_alive(pid: int) -> bool:
    try:
        pid = int(pid)
    except (TypeError, ValueError):
        return False
    if pid <= 0:
        return False
    handle = kernel32.OpenProcess(SYNCHRONIZE, False, pid)
    if not handle:
        return False
    try:
        return kernel32.WaitForSingleObject(handle, 0) == WAIT_TIMEOUT
    finally:
        kernel32.CloseHandle(handle)


def proc_memory(pid: int):
    """返回 (工作集字节, 提交内存字节)。"""
    if not pid:
        return 0, 0
    handle = kernel32.OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, False, int(pid))
    limited = False
    if not handle:
        handle = kernel32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, int(pid))
        limited = True
    if not handle:
        return 0, 0
    try:
        counters = PROCESS_MEMORY_COUNTERS_EX()
        counters.cb = ctypes.sizeof(counters)
        if not psapi.GetProcessMemoryInfo(handle, ctypes.byref(counters), counters.cb):
            return 0, 0
        private = counters.PrivateUsage if not limited else counters.PagefileUsage
        return int(counters.WorkingSetSize), int(private)
    finally:
        kernel32.CloseHandle(handle)


def proc_create_time(pid: int) -> Optional[datetime]:
    if not pid:
        return None
    handle = kernel32.OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, False, int(pid))
    if not handle:
        return None
    try:
        creation = FILETIME()
        exited = FILETIME()
        kernel = FILETIME()
        user = FILETIME()
        if not kernel32.GetProcessTimes(
            handle,
            ctypes.byref(creation),
            ctypes.byref(exited),
            ctypes.byref(kernel),
            ctypes.byref(user),
        ):
            return None
        ticks = (int(creation.dwHighDateTime) << 32) | int(creation.dwLowDateTime)
        if ticks <= 0:
            return None
        epoch_seconds = (ticks - 116444736000000000) / 10_000_000
        try:
            return datetime.fromtimestamp(epoch_seconds)
        except (OverflowError, OSError, ValueError):
            return None
    finally:
        kernel32.CloseHandle(handle)


def build_env_block(env: Dict[str, str]):
    """把 dict 转成 CreateProcessW 需要的 Unicode 环境块（双 NUL 结尾）。"""
    items = [f"{key}={value}" for key, value in env.items() if key and "=" not in key]
    raw = "\0".join(items) + "\0\0"
    return ctypes.create_unicode_buffer(raw, len(raw) + 1)


def clean_launch_env() -> Dict[str, str]:
    """
    构造干净的子进程环境：
    - 去掉 ELECTRON_RUN_AS_NODE：部分 IDE/自动化框架会注入它，导致 Electron 以 Node 模式运行后秒退；
    - 去掉 ELECTRON_NO_ATTACH_CONSOLE / MUW_CMD_LOG：避免把看门狗自身的控制台/日志句柄传给子进程；
    - 去掉 PyInstaller 的 _PYI_* / PYINSTALLER_* 内部变量：
      否则 onefile exe 再启动自身时，新进程会继承父进程的临时解包目录信息，导致 stdlib 行为异常。
    """
    blocked = {"ELECTRON_RUN_AS_NODE", "ELECTRON_NO_ATTACH_CONSOLE", "MUW_CMD_LOG"}
    cleaned: Dict[str, str] = {}
    for key, value in os.environ.items():
        upper = key.upper()
        if upper in blocked or upper.startswith("_PYI_") or upper.startswith("PYINSTALLER_"):
            continue
        cleaned[key] = value
    return cleaned


def win32_spawn(
    exe: os.PathLike,
    args: Optional[List[str]] = None,
    cwd: Optional[os.PathLike] = None,
    env: Optional[Dict[str, str]] = None,
    hidden: bool = False,
    detached: bool = False,
) -> int:
    """用 Win32 CreateProcessW 启动进程并返回 PID。"""
    executable = str(exe)
    command_line_text = subprocess.list2cmdline([executable] + [str(item) for item in (args or [])])
    command_line = ctypes.create_unicode_buffer(command_line_text)
    flags = CREATE_UNICODE_ENVIRONMENT
    if hidden:
        flags |= 0x08000000  # CREATE_NO_WINDOW
    if detached:
        flags |= 0x00000008  # DETACHED_PROCESS
    environment = build_env_block(env if env is not None else os.environ)
    startup = STARTUPINFOW()
    startup.cb = ctypes.sizeof(STARTUPINFOW)
    if hidden:
        startup.dwFlags |= STARTF_USESHOWWINDOW
        startup.wShowWindow = SW_HIDE
    process_info = PROCESS_INFORMATION()
    ok = kernel32.CreateProcessW(
        None,
        ctypes.cast(command_line, wt.LPWSTR),
        None,
        None,
        False,
        flags,
        ctypes.cast(environment, ctypes.c_void_p),
        str(cwd) if cwd else None,
        ctypes.byref(startup),
        ctypes.byref(process_info),
    )
    if not ok:
        raise ctypes.WinError(ctypes.get_last_error())
    pid = int(process_info.dwProcessId)
    if process_info.hThread:
        kernel32.CloseHandle(process_info.hThread)
    if process_info.hProcess:
        kernel32.CloseHandle(process_info.hProcess)
    return pid


def attach_parent_console() -> bool:
    """窗口化 exe 从命令行启动时，附加到父控制台。"""
    try:
        return bool(kernel32.AttachConsole(ATTACH_PARENT_PROCESS))
    except Exception:
        return False


__all__ = [
    "ProcessInfo",
    "list_processes",
    "proc_exe",
    "is_pid_alive",
    "proc_memory",
    "proc_create_time",
    "build_env_block",
    "clean_launch_env",
    "win32_spawn",
    "attach_parent_console",
]
