//! Win32 API 封装：进程枚举、路径、存活判断、内存/句柄统计、控制台附着、启动时间。

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use chrono::{DateTime, Local, TimeZone};
use windows_sys::Win32::Foundation::{
    CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Console::{AttachConsole, GetConsoleWindow, ATTACH_PARENT_PROCESS};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetProcessHandleCount, GetProcessTimes, OpenProcess,
    QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
};

const SYNCHRONIZE: u32 = 0x0010_0000;
const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
const PROCESS_VM_READ: u32 = 0x0010;

#[derive(Debug, Clone)]
pub struct ProcessEntry {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
}

/// Toolhelp32 枚举全部进程。
pub fn list_processes() -> Vec<ProcessEntry> {
    let mut result = Vec::new();
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return result;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snapshot, &mut entry) != 0;
        while ok {
            let name_end = entry
                .szExeFile
                .iter()
                .position(|ch| *ch == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..name_end]);
            result.push(ProcessEntry {
                pid: entry.th32ProcessID,
                ppid: entry.th32ParentProcessID,
                name,
            });
            ok = Process32NextW(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
    }
    result
}

/// 查询进程可执行文件完整路径。
pub fn proc_exe(pid: u32) -> Option<PathBuf> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut buffer = vec![0u16; 32768];
        let mut size = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut size) != 0;
        CloseHandle(handle);
        if !ok {
            return None;
        }
        buffer.truncate(size as usize);
        let path = OsString::from_wide(&buffer);
        Some(PathBuf::from(path))
    }
}

pub fn is_pid_alive(pid: u32) -> bool {
    unsafe {
        let handle = OpenProcess(SYNCHRONIZE, 0, pid);
        if handle.is_null() {
            return false;
        }
        let wait = windows_sys::Win32::System::Threading::WaitForSingleObject(handle, 0);
        CloseHandle(handle);
        wait == WAIT_TIMEOUT
    }
}

pub fn attach_parent_console() -> bool {
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) != 0 }
}

pub fn console_available() -> bool {
    unsafe { !GetConsoleWindow().is_null() }
}

/// 当前进程 (工作集字节, 提交内存字节)。
pub fn self_memory() -> (u64, u64) {
    unsafe { memory_from_handle(GetCurrentProcess()).unwrap_or((0, 0)) }
}

fn memory_from_handle(handle: HANDLE) -> Option<(u64, u64)> {
    if handle.is_null() {
        return None;
    }
    unsafe {
        let mut counters: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        let ok = GetProcessMemoryInfo(handle, &mut counters, counters.cb) != 0;
        if !ok {
            return None;
        }
        Some((counters.WorkingSetSize as u64, counters.PagefileUsage as u64))
    }
}

pub fn process_memory(pid: u32) -> Option<(u64, u64)> {
    unsafe {
        let mut handle = OpenProcess(
            PROCESS_QUERY_INFORMATION | PROCESS_VM_READ,
            0,
            pid,
        );
        if handle.is_null() {
            handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        }
        if handle.is_null() {
            return None;
        }
        let result = memory_from_handle(handle);
        CloseHandle(handle);
        result
    }
}

/// 当前进程句柄数。
pub fn self_handle_count() -> u32 {
    unsafe {
        let mut count: u32 = 0;
        if GetProcessHandleCount(GetCurrentProcess(), &mut count) != 0 {
            count
        } else {
            0
        }
    }
}

fn filetime_ticks(value: FILETIME) -> u64 {
    ((value.dwHighDateTime as u64) << 32) | value.dwLowDateTime as u64
}

/// 当前进程累计 CPU 时间（秒，kernel + user）。
pub fn self_cpu_seconds() -> Option<f64> {
    unsafe {
        let mut creation: FILETIME = std::mem::zeroed();
        let mut exit: FILETIME = std::mem::zeroed();
        let mut kernel: FILETIME = std::mem::zeroed();
        let mut user: FILETIME = std::mem::zeroed();
        let ok = GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        ) != 0;
        if !ok {
            return None;
        }
        let ticks = filetime_ticks(kernel) + filetime_ticks(user);
        Some(ticks as f64 / 10_000_000.0)
    }
}

/// 进程创建时间（本地时间）。
pub fn proc_create_time(pid: u32) -> Option<DateTime<Local>> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut creation: FILETIME = std::mem::zeroed();
        let mut exit: FILETIME = std::mem::zeroed();
        let mut kernel: FILETIME = std::mem::zeroed();
        let mut user: FILETIME = std::mem::zeroed();
        let ok = GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) != 0;
        CloseHandle(handle);
        if !ok {
            return None;
        }
        let ticks = ((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64;
        if ticks == 0 {
            return None;
        }
        let seconds = (ticks as i64 - 116_444_736_000_000_000) / 10_000_000;
        Local.timestamp_opt(seconds, 0).single()
    }
}

/// 进程 PID 的父进程号（从快照中查）。
pub fn parent_pid(pid: u32) -> Option<u32> {
    list_processes()
        .into_iter()
        .find(|entry| entry.pid == pid)
        .map(|entry| entry.ppid)
}

/// 等待子进程退出（带超时，超时则终止进程树）。
pub fn wait_with_timeout(pid: u32, timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if !is_pid_alive(pid) {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            let _ = crate::util::taskkill_tree(pid);
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
