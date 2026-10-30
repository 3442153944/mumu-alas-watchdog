//! CLI / GUI 输出：支持 MUW_CMD_LOG 捕获（GUI 调用子命令时）。

use std::fs::OpenOptions;
use std::io::Write;

pub fn emit(line: &str) {
    if let Ok(path) = std::env::var("MUW_CMD_LOG") {
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "{line}");
        }
        return;
    }
    // 窗口化 exe 在没有控制台时写 stdout 会 panic；此时静默（日志仍会落盘）。
    if crate::win32::console_available() {
        println!("{line}");
    }
}

#[macro_export]
macro_rules! outln {
    ($($arg:tt)*) => {
        $crate::output::emit(&format!($($arg)*))
    };
}
