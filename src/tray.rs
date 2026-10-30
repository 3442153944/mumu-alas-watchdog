//! 系统托盘：守护进程无任务栏窗口，但常驻托盘图标 + 右键菜单。
//!
//! 实现方式：一个隐藏的 Win32 窗口接收 Shell_NotifyIcon 回调消息，
//! 菜单位于该窗口消息循环中处理；托盘线程与看门狗主循环互不阻塞。

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::Duration;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DispatchMessageW, GetCursorPos, GetMessageW, LoadIconW, LoadImageW, PostMessageW, PostQuitMessage,
    RegisterClassW, SetForegroundWindow, SetTimer, TrackPopupMenu, TranslateMessage, CS_DBLCLKS,
    IDI_APPLICATION, IMAGE_ICON, LR_LOADFROMFILE, MF_SEPARATOR, MF_STRING, MSG, TPM_NONOTIFY,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_APP, WM_CONTEXTMENU, WM_DESTROY, WM_LBUTTONDBLCLK, WM_NULL,
    WM_RBUTTONUP, WM_TIMER, WNDCLASSW, WS_OVERLAPPED,
};

use crate::constants;
use crate::util;

const WM_TRAY_CALLBACK: u32 = WM_APP + 1;
const WM_TRAY_SHUTDOWN: u32 = WM_APP + 2;
const MENU_OPEN_CONSOLE: usize = 1;
const MENU_RESTART_ALL: usize = 2;
const MENU_RESTART_ALAS: usize = 3;
const MENU_OPEN_LOGS: usize = 4;
const MENU_OPEN_CONFIG: usize = 5;
const MENU_EXIT: usize = 6;
const TIMER_UPDATE_TIP: usize = 1;

static TRAY_TIP: OnceLock<Arc<Mutex<String>>> = OnceLock::new();
static TRAY_HWND: AtomicIsize = AtomicIsize::new(0);

/// 更新托盘悬浮提示（供看门狗状态刷新时调用；未启动托盘时是空操作）。
pub fn set_tip(text: &str) {
    if let Some(tip) = TRAY_TIP.get() {
        if let Ok(mut guard) = tip.lock() {
            *guard = text.to_string();
        }
    }
}

pub struct Tray {
    hwnd: isize,
    thread: Option<std::thread::JoinHandle<()>>,
}

unsafe impl Send for Tray {}

impl Tray {
    pub fn start(initial_tip: &str) -> Result<Tray, String> {
        let tip = Arc::new(Mutex::new(initial_tip.to_string()));
        let _ = TRAY_TIP.set(tip.clone());
        let (sender, receiver) = mpsc::channel::<isize>();
        let thread = std::thread::Builder::new()
            .name("tray".to_string())
            .spawn(move || {
                if let Err(err) = tray_thread(sender.clone()) {
                    eprintln!("托盘初始化失败: {err}");
                    let _ = sender.send(0);
                }
            })
            .map_err(|err| format!("创建托盘线程失败: {err}"))?;
        match receiver.recv_timeout(Duration::from_secs(5)) {
            Ok(0) => Err("托盘窗口创建失败".to_string()),
            Ok(hwnd) => {
                TRAY_HWND.store(hwnd, Ordering::SeqCst);
                Ok(Tray {
                    hwnd,
                    thread: Some(thread),
                })
            }
            Err(err) => Err(format!("等待托盘初始化超时: {err}")),
        }
    }

    pub fn shutdown(&mut self) {
        if let Some(thread) = self.thread.take() {
            unsafe {
                PostMessageW(self.hwnd as HWND, WM_TRAY_SHUTDOWN, 0, 0);
            }
            // 托盘线程可能正停留在右键菜单的模态循环中；最多等 3 秒，避免退出卡死。
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while !thread.is_finished() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
            if thread.is_finished() {
                let _ = thread.join();
            }
            TRAY_HWND.store(0, Ordering::SeqCst);
        }
    }
}

fn wide(text: &str) -> Vec<u16> {
    OsStr::new(text).encode_wide().chain(std::iter::once(0)).collect()
}

fn path_wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
}

fn tray_thread(sender: mpsc::Sender<isize>) -> Result<(), String> {
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let class_name = wide("MuMuAlasWatchdogTray");
        let window_title = wide("MuMuAlasWatchdogTray");
        let icon = load_tray_icon(instance);
        let window_class = WNDCLASSW {
            style: CS_DBLCLKS,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance as _,
            hIcon: icon,
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class_name.as_ptr(),
        };
        if RegisterClassW(&window_class) == 0 {
            return Err(format!(
                "RegisterClassW 失败: {}",
                std::io::Error::last_os_error()
            ));
        }
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            window_title.as_ptr(),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            instance as _,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return Err(format!(
                "CreateWindowExW 失败: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mut data: NOTIFYICONDATAW = std::mem::zeroed();
        data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = hwnd;
        data.uID = 1;
        data.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        data.uCallbackMessage = WM_TRAY_CALLBACK;
        data.hIcon = icon;
        let tip_text = wide(
            TRAY_TIP
                .get()
                .and_then(|tip| tip.lock().ok().map(|guard| guard.clone()))
                .unwrap_or_else(|| "MuMu + Alas 看门狗".to_string())
                .as_str(),
        );
        copy_tip(&mut data.szTip, &tip_text);
        if Shell_NotifyIconW(NIM_ADD, &data) == 0 {
            return Err("Shell_NotifyIcon(NIM_ADD) 失败".to_string());
        }
        SetTimer(hwnd, TIMER_UPDATE_TIP, 10_000, None);
        let _ = sender.send(hwnd as isize);

        let mut message: MSG = std::mem::zeroed();
        while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        Shell_NotifyIconW(NIM_DELETE, &data);
        if !icon.is_null() {
            destroy_icon_safe(icon);
        }
    }
    Ok(())
}

fn copy_tip(destination: &mut [u16; 128], text: &[u16]) {
    for (index, value) in destination.iter_mut().enumerate() {
        *value = text.get(index).copied().unwrap_or(0);
    }
    destination[127] = 0;
}

fn load_tray_icon(instance: *mut std::ffi::c_void) -> *mut std::ffi::c_void {
    unsafe {
        let icon_path = constants::wd_dir().join("icon.ico");
        let icon = LoadImageW(
            instance as _,
            path_wide(&icon_path).as_ptr(),
            IMAGE_ICON,
            32,
            32,
            LR_LOADFROMFILE,
        );
        if !icon.is_null() {
            icon
        } else {
            LoadIconW(instance as _, IDI_APPLICATION) as *mut std::ffi::c_void
        }
    }
}

unsafe fn destroy_icon_safe(icon: *mut std::ffi::c_void) {
    if !icon.is_null() {
        windows_sys::Win32::UI::WindowsAndMessaging::DestroyIcon(icon);
    }
}

unsafe fn update_tooltip(hwnd: HWND) {
    let Some(tip) = TRAY_TIP.get() else {
        return;
    };
    let Ok(text) = tip.lock() else {
        return;
    };
    let mut data: NOTIFYICONDATAW = std::mem::zeroed();
    data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = 1;
    data.uFlags = NIF_TIP;
    let wide_tip = wide(text.as_str());
    copy_tip(&mut data.szTip, &wide_tip);
    Shell_NotifyIconW(NIM_MODIFY, &data);
}

unsafe extern "system" fn wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_TRAY_CALLBACK => {
            let event = (lparam as u32) & 0xFFFF;
            if event == WM_RBUTTONUP || event == WM_CONTEXTMENU {
                show_menu(hwnd);
            } else if event == WM_LBUTTONDBLCLK {
                open_console();
            }
            0
        }
        WM_TIMER if wparam == TIMER_UPDATE_TIP => {
            update_tooltip(hwnd);
            0
        }
        WM_TRAY_SHUTDOWN => {
            let mut data: NOTIFYICONDATAW = std::mem::zeroed();
            data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            data.hWnd = hwnd;
            data.uID = 1;
            Shell_NotifyIconW(NIM_DELETE, &data);
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

unsafe fn append(menu: *mut std::ffi::c_void, flags: u32, id: usize, text: &str) {
    let wide_text = wide(text);
    AppendMenuW(menu, flags, id, wide_text.as_ptr());
}

unsafe fn show_menu(hwnd: HWND) {
    let menu = CreatePopupMenu();
    if menu.is_null() {
        return;
    }
    append(menu, MF_STRING, MENU_OPEN_CONSOLE, "打开控制台");
    append(menu, MF_STRING, MENU_RESTART_ALL, "立即完整重启");
    append(menu, MF_STRING, MENU_RESTART_ALAS, "只重启 Alas");
    append(menu, MF_SEPARATOR, 0, "");
    append(menu, MF_STRING, MENU_OPEN_LOGS, "打开日志目录");
    append(menu, MF_STRING, MENU_OPEN_CONFIG, "打开配置文件");
    append(menu, MF_SEPARATOR, 0, "");
    append(menu, MF_STRING, MENU_EXIT, "退出看门狗");

    let mut point: POINT = std::mem::zeroed();
    GetCursorPos(&mut point);
    SetForegroundWindow(hwnd);
    let command = TrackPopupMenu(
        menu,
        TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY,
        point.x,
        point.y,
        0,
        hwnd,
        std::ptr::null(),
    ) as usize;
    DestroyMenu(menu);
    PostMessageW(hwnd, WM_NULL, 0, 0);

    match command {
        MENU_OPEN_CONSOLE => open_console(),
        MENU_RESTART_ALL => {
            let _ = std::fs::write(constants::flag_full_restart(), util::now_text());
        }
        MENU_RESTART_ALAS => {
            let _ = std::fs::write(constants::flag_alas_restart(), util::now_text());
        }
        MENU_OPEN_LOGS => {
            let _ = std::process::Command::new("explorer")
                .arg(constants::wd_dir().join("logs"))
                .spawn();
        }
        MENU_OPEN_CONFIG => {
            let _ = std::process::Command::new("notepad")
                .arg(constants::config_path())
                .spawn();
        }
        MENU_EXIT => {
            let _ = std::fs::write(constants::flag_stop(), util::now_text());
        }
        _ => {}
    }
}

fn open_console() {
    let args = vec!["--gui".to_string()];
    let _ = util::spawn_process(
        constants::app_path(),
        &args,
        Some(constants::wd_dir()),
        false,
        false,
        false,
    );
}

pub fn running() -> bool {
    TRAY_HWND.load(Ordering::SeqCst) != 0
}
