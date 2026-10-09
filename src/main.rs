#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

//! 程序入口与模块装配。
//!
//! 这里声明应用、界面组件、领域逻辑、平台能力和视觉模块，并集中提供少量兼容性
//! 重导出。`main` 本身只调用 [`app::run`]，不承载具体业务或绘制逻辑。

mod ai;
mod app;
mod components;
mod domain;
mod i18n;
mod platform;
mod ui;

pub(crate) use app::{
    UiConfig, WORD_EDITOR_FONT_FAMILY, project_directory, word_library_directory,
};
pub(crate) use components::word_library::editor as word_editor;
pub(crate) use domain::translation_language;
pub(crate) use ui::theme as ui_theme;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|argument| argument == "--migrate-data") {
        let statistics = domain::storage::library_statistics()?;
        println!("SQLite: {}", domain::storage::database_path().display());
        for (library, active, quarantined) in statistics {
            println!("{library}: {active} active, {quarantined} quarantined");
        }
        println!("隔离总数: {}", domain::storage::quarantine_count()?);
        return Ok(());
    }
    app::run()?;
    Ok(())
}
