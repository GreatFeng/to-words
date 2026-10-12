#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

//! 程序入口。共用业务模块在 `lib.rs`，便于 React 与旧版界面迁移期间复用。

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|argument| argument == "--migrate-data") {
        hotkey_word_match::migrate_data_report()?;
        return Ok(());
    }
    #[cfg(feature = "react-ui")]
    hotkey_word_match::run_react()?;
    #[cfg(not(feature = "react-ui"))]
    hotkey_word_match::run_egui()?;
    Ok(())
}
