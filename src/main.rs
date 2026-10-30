#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    std::process::exit(mumu_alas_watchdog_rs::cli::run());
}
