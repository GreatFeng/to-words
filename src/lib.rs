//! to_words 共享 Rust 核心。
//!
//! 原生平台能力、词库与配置逻辑由两种界面运行时复用；React 迁移完成前，
//! 现有 egui 入口仍可独立运行。

mod ai;
#[cfg(not(feature = "react-ui"))]
mod app;
#[cfg(not(feature = "react-ui"))]
mod components;
mod domain;
mod i18n;
mod platform;
#[cfg(not(feature = "react-ui"))]
mod ui;
#[cfg(feature = "react-ui")]
mod web_config;

#[cfg(feature = "react-ui")]
mod web_app;

#[cfg(not(feature = "react-ui"))]
pub(crate) use app::{
    UiConfig, WORD_EDITOR_FONT_FAMILY, project_directory, word_library_directory,
};
#[cfg(not(feature = "react-ui"))]
pub(crate) use components::word_library::editor as word_editor;
pub(crate) use domain::translation_language;
#[cfg(not(feature = "react-ui"))]
pub(crate) use ui::theme as ui_theme;
#[cfg(feature = "react-ui")]
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

#[cfg(not(feature = "react-ui"))]
pub fn run_egui() -> eframe::Result {
    app::run()
}

#[cfg(feature = "react-ui")]
pub fn run_react() -> anyhow::Result<()> {
    web_app::run()
}
