#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

//! 程序入口。界面使用 React/Tauri，业务模块位于 `lib.rs`。

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|argument| argument == "--migrate-data") {
        hotkey_word_match::migrate_data_report()?;
        return Ok(());
    }
    hotkey_word_match::run_react()?;
    Ok(())
}
