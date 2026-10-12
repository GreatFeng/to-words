//! React 运行时使用的设置模型和路径。迁移期与旧版配置保持相同的 JSON 字段。

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::ai::PolitenessLevel;
use crate::domain::{storage, translation_language};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct UiConfig {
    pub(crate) input: InputConfig,
    pub(crate) results: ResultsConfig,
    pub(crate) hotkeys: HotkeyConfig,
    pub(crate) gap: f32,
    pub(crate) continuous_input: bool,
    pub(crate) ai_translation: bool,
    pub(crate) ai_auto_save: bool,
    #[serde(alias = "ai_polite_mode", alias = "ai_korean_honorific")]
    pub(crate) ai_politeness: PolitenessLevel,
    pub(crate) ocr_auto_translate: bool,
    pub(crate) ocr_show_original: bool,
    pub(crate) voice_auto_copy_first: bool,
    pub(crate) voice_aion2_manual_paste: bool,
    pub(crate) voice_keep_input: bool,
    pub(crate) voice_hold_to_talk: bool,
    pub(crate) voice_owner_filter: bool,
    pub(crate) voice_playback_filter: bool,
    pub(crate) voice_silence_seconds: f32,
    pub(crate) voice_backend: String,
    pub(crate) voice_language: String,
    pub(crate) clear_on_focus_return: bool,
    pub(crate) translation_language: String,
    pub(crate) source_language: String,
    pub(crate) ui_language: String,
    pub(crate) text_color: String,
    pub(crate) selected_color: String,
    pub(crate) selected_text_color: String,
    pub(crate) selected_opacity: f32,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            input: InputConfig::default(),
            results: ResultsConfig::default(),
            hotkeys: HotkeyConfig::default(),
            gap: 4.0,
            continuous_input: false,
            ai_translation: false,
            ai_auto_save: true,
            ai_politeness: PolitenessLevel::Informal,
            ocr_auto_translate: true,
            ocr_show_original: true,
            voice_auto_copy_first: true,
            voice_aion2_manual_paste: false,
            voice_keep_input: false,
            voice_hold_to_talk: false,
            voice_owner_filter: false,
            voice_playback_filter: false,
            voice_silence_seconds: 0.6,
            voice_backend: "sensevoice".into(),
            voice_language: "auto".into(),
            clear_on_focus_return: false,
            translation_language: String::new(),
            source_language: "auto".into(),
            ui_language: "zh-CN".into(),
            text_color: "#24272E".into(),
            selected_color: "#A9CEFF".into(),
            selected_text_color: "#0B57D0".into(),
            selected_opacity: 0.72,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct InputConfig {
    pub(crate) background_color: String,
    pub(crate) opacity: f32,
    pub(crate) width: f32,
    pub(crate) height: f32,
    pub(crate) font_size: f32,
    pub(crate) corner_radius: f32,
}

impl Default for InputConfig {
    fn default() -> Self {
        Self {
            background_color: "#E8E8E8".into(),
            opacity: 0.20,
            width: 200.0,
            height: 30.0,
            font_size: 14.0,
            corner_radius: 9.0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct ResultsConfig {
    pub(crate) background_color: String,
    pub(crate) opacity: f32,
    pub(crate) min_width: f32,
    pub(crate) max_width: f32,
    pub(crate) min_row_height: f32,
    pub(crate) max_height: f32,
    pub(crate) font_size: f32,
    pub(crate) corner_radius: f32,
    pub(crate) max_visible_results: usize,
}

impl Default for ResultsConfig {
    fn default() -> Self {
        Self {
            background_color: "#E8E8E8".into(),
            opacity: 0.20,
            min_width: 200.0,
            max_width: 600.0,
            min_row_height: 28.0,
            max_height: 360.0,
            font_size: 12.0,
            corner_radius: 7.0,
            max_visible_results: 8,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct HotkeyConfig {
    pub(crate) popup: String,
    pub(crate) settings: String,
    pub(crate) clear_composed: String,
    pub(crate) ocr: String,
    pub(crate) voice: String,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            popup: "ctrl+alt+enter".into(),
            settings: "ctrl+alt+s".into(),
            clear_composed: "ctrl+backspace".into(),
            ocr: "ctrl+alt+o".into(),
            voice: "ctrl+alt+v".into(),
        }
    }
}

impl UiConfig {
    pub(crate) fn normalized(mut self) -> Self {
        self.input.opacity = self.input.opacity.clamp(0.0, 1.0);
        self.input.width = self.input.width.clamp(80.0, 1200.0);
        self.input.height = self.input.height.clamp(18.0, 200.0);
        self.input.font_size = self.input.font_size.clamp(8.0, 72.0);
        self.input.corner_radius = self.input.corner_radius.clamp(0.0, 100.0);
        self.results.opacity = self.results.opacity.clamp(0.0, 1.0);
        self.results.min_width = self.results.min_width.clamp(80.0, 1200.0);
        self.results.max_width = self.results.max_width.clamp(self.results.min_width, 1600.0);
        self.results.min_row_height = self.results.min_row_height.clamp(18.0, 200.0);
        self.results.max_height = self.results.max_height.clamp(40.0, 1200.0);
        self.results.font_size = self.results.font_size.clamp(8.0, 72.0);
        self.results.corner_radius = self.results.corner_radius.clamp(0.0, 100.0);
        self.results.max_visible_results = self.results.max_visible_results.clamp(1, 100);
        if !self.voice_silence_seconds.is_finite() {
            self.voice_silence_seconds = 0.6;
        }
        self.voice_silence_seconds =
            (self.voice_silence_seconds.clamp(0.2, 3.0) * 10.0).round() / 10.0;
        self.gap = self.gap.clamp(0.0, 40.0);
        self.selected_opacity = self.selected_opacity.clamp(0.0, 1.0);
        self.translation_language =
            translation_language::normalize_language_code(&self.translation_language);
        self.source_language = translation_language::normalize_source_code(&self.source_language);
        self.ui_language = crate::i18n::UiLanguage::from_code(&self.ui_language)
            .code()
            .to_string();
        self.voice_language = translation_language::normalize_source_code(&self.voice_language);
        if !matches!(self.voice_backend.as_str(), "sensevoice" | "whisper") {
            self.voice_backend = "sensevoice".into();
        }
        self
    }
}

pub(crate) fn project_directory() -> PathBuf {
    let working = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if working.join("ui_config.json").exists()
        || working.join(storage::DATABASE_FILE).exists()
        || working.join("user_words.json").exists()
        || working.join("word_libraries").exists()
    {
        return working;
    }
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or(working)
}

pub(crate) fn word_library_directory() -> PathBuf {
    project_directory().join("word_libraries")
}

pub(crate) fn load_ui_config() -> Result<UiConfig> {
    let Some(content) = storage::load_setting("ui_config")? else {
        return Ok(UiConfig::default());
    };
    let config: UiConfig = serde_json::from_str(&content).context("数据库中的 UI 设置格式错误")?;
    Ok(config.normalized())
}

pub(crate) fn save_ui_config(config: &UiConfig) -> Result<()> {
    let content = serde_json::to_string_pretty(config).context("无法生成 UI 配置")?;
    storage::save_setting("ui_config", &content)
}

#[cfg(windows)]
pub(crate) fn web_query_position(width: f32, height: f32, scale: f32) -> Option<(i32, i32)> {
    crate::platform::caret_position::query_position(width, height, scale)
}

#[cfg(not(windows))]
pub(crate) fn web_query_position(_width: f32, _height: f32, _scale: f32) -> Option<(i32, i32)> {
    None
}

#[cfg(test)]
mod tests {
    use super::UiConfig;
    use crate::ai::PolitenessLevel;

    #[test]
    fn old_polite_switch_migrates_to_conversational_and_new_level_round_trips() {
        let old: UiConfig = serde_json::from_str(r#"{"ai_polite_mode":true}"#).unwrap();
        assert_eq!(old.ai_politeness, PolitenessLevel::Conversational);
        let older: UiConfig = serde_json::from_str(r#"{"ai_korean_honorific":false}"#).unwrap();
        assert_eq!(older.ai_politeness, PolitenessLevel::Informal);
        let formal: UiConfig = serde_json::from_str(r#"{"ai_politeness":"formal"}"#).unwrap();
        assert_eq!(formal.ai_politeness, PolitenessLevel::Formal);
        let saved = serde_json::to_value(formal).unwrap();
        assert_eq!(saved["ai_politeness"], "formal");
        assert!(saved.get("ai_polite_mode").is_none());
    }

    #[test]
    fn ocr_original_display_defaults_on_and_persists_when_disabled() {
        let old: UiConfig = serde_json::from_str("{}").unwrap();
        assert!(old.ocr_show_original);
        let hidden: UiConfig = serde_json::from_str(r#"{"ocr_show_original":false}"#).unwrap();
        assert!(!hidden.ocr_show_original);
        let saved = serde_json::to_value(hidden).unwrap();
        assert_eq!(saved["ocr_show_original"], false);
    }
}
