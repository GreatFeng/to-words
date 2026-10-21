//! to_words Rust 核心与 React/Tauri 桌面运行时。

mod ai;
mod domain;
mod i18n;
mod platform;
mod web_app;
mod web_config;

pub(crate) use domain::translation_language;
pub(crate) use web_config::{project_directory, word_library_directory};

pub fn migrate_data_report() -> anyhow::Result<()> {
    let statistics = domain::storage::library_statistics()?;
    println!("SQLite: {}", domain::storage::database_path().display());
    for (library, active, quarantined) in statistics {
        println!("{library}: {active} active, {quarantined} quarantined");
    }
    println!("隔离总数: {}", domain::storage::quarantine_count()?);
    Ok(())
}

pub fn run_react() -> anyhow::Result<()> {
    web_app::run()
}
