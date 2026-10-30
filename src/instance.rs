//! 系统互斥锁在进程退出后自动释放；GUI 与后台分别加锁，以便同时运行。
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE},
    System::Threading::CreateMutexW,
};

pub struct InstanceGuard(HANDLE);
impl InstanceGuard {
    pub fn acquire(role: &str) -> Result<Option<Self>, String> {
        let name: Vec<u16> = format!("Local\\MuMuAlasWatchdog.{role}")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        unsafe {
            let handle = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            if handle.is_null() {
                return Err(format!(
                    "创建实例锁失败：{}",
                    std::io::Error::last_os_error()
                ));
            }
            if GetLastError() == ERROR_ALREADY_EXISTS {
                CloseHandle(handle);
                return Ok(None);
            }
            Ok(Some(Self(handle)))
        }
    }
}
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

pub fn restore_gui() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE,
    };
    let title: Vec<u16> = format!(
        "{} v{} · MuMu + Alas 自动重启控制台",
        crate::constants::APP_NAME,
        crate::constants::VERSION
    )
    .encode_utf16()
    .chain(Some(0))
    .collect();
    unsafe {
        let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
        if hwnd.is_null() {
            return false;
        }
        ShowWindow(hwnd, SW_RESTORE);
        SetForegroundWindow(hwnd);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn subprocess_probe() {
        if let Ok(role) = std::env::var("MUW_TEST_MUTEX_ROLE") {
            assert!(InstanceGuard::acquire(&role).unwrap().is_none());
        }
    }
    #[test]
    fn excludes_another_process_and_releases_on_drop() {
        let role = format!("test.{}", std::process::id());
        let guard = InstanceGuard::acquire(&role).unwrap().unwrap();
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "instance::tests::subprocess_probe"])
            .env("MUW_TEST_MUTEX_ROLE", &role)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        drop(guard);
        assert!(InstanceGuard::acquire(&role).unwrap().is_some());
    }
}
