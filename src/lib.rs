//! MuMu 模拟器 + Alas 自动重启看门狗（Rust + egui）。

pub mod alas;
pub mod appearance;
pub mod cli;
pub mod config;
pub mod config_editor;
pub mod constants;
pub mod emulator;
pub mod gui;
pub mod integrations;
pub mod instance;
pub mod reload;
pub mod layout;
pub mod logger;
pub mod logwatch;
pub mod output;
pub mod schedule;
pub mod status;
pub mod tray;
pub mod util;
pub mod watchdog;
pub mod win32;
