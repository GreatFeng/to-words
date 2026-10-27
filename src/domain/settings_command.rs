//! Voice command allowlist: validate AI proposals before changing persisted settings.

use crate::ai::PolitenessLevel;
use crate::domain::shortcut::Shortcut;
use crate::domain::translation_language;
use crate::web_config::{SELECTED_TEXT_TRANSLATION_ACTIVE, UiConfig};
use anyhow::{Result, bail};
use serde_json::Value;
use std::collections::HashSet;

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub(crate) struct SettingChange {
    pub(crate) setting: String,
    pub(crate) value: Value,
    #[serde(default)]
    pub(crate) feedback: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CommandAction {
    OpenOcr,
    OpenQuery,
    OpenSettings,
    StartVoice,
    MicTts(Option<bool>),
    SystemAudio(Option<bool>),
    TranslateText(String),
    TranslateSelection,
}

/// Only explicitly supported immediate operations may leave the settings queue.
pub(crate) fn parse_action(change: &SettingChange) -> Result<Option<CommandAction>> {
    let no_value = || {
        if change.value.is_null() {
            Ok(())
        } else {
            bail!("该操作不需要附加内容")
        }
    };
    let switch = || match &change.value {
        Value::Bool(enabled) => Ok(Some(*enabled)),
        Value::String(value) if value == "toggle" => Ok(None),
        _ => bail!("开关操作需要 true、false 或 toggle"),
    };
    let action = match change.setting.as_str() {
        "action.open_ocr" => {
            no_value()?;
            CommandAction::OpenOcr
        }
        "action.open_query" => {
            no_value()?;
            CommandAction::OpenQuery
        }
        "action.open_settings" => {
            no_value()?;
            CommandAction::OpenSettings
        }
        "action.start_voice" => {
            no_value()?;
            CommandAction::StartVoice
        }
        "action.mic_tts" => CommandAction::MicTts(switch()?),
        "action.system_audio" => CommandAction::SystemAudio(switch()?),
        "action.translate_selection" => {
            if !SELECTED_TEXT_TRANSLATION_ACTIVE {
                bail!("选中文字翻译已暂时停用");
            }
            no_value()?;
            CommandAction::TranslateSelection
        }
        "action.translate_text" => {
            let text = change.value.as_str().unwrap_or_default().trim();
            if text.is_empty() || text.chars().count() > 4_000 {
                bail!("请说出不超过 4000 字的待翻译文字");
            }
            CommandAction::TranslateText(text.to_owned())
        }
        value if value.starts_with("action.") => bail!("不支持的即时操作：{value}"),
        _ => return Ok(None),
    };
    Ok(Some(action))
}

/// Only exact exit phrases bypass AI, so a negated sentence cannot close the mode.
pub(crate) fn is_exit_phrase(text: &str, language: &str) -> bool {
    let spoken = text
        .trim_matches(|ch: char| ch.is_whitespace() || "。.!！？?，,".contains(ch))
        .to_lowercase();
    match language {
        "cn" | "auto" | "zh_tw" => matches!(
            spoken.replace(' ', "").as_str(),
            "关闭指令模式"
                | "退出指令模式"
                | "结束指令模式"
                | "停止指令模式"
                | "关闭命令模式"
                | "退出命令模式"
                | "结束命令模式"
        ),
        "en" => matches!(
            spoken.as_str(),
            "stop command mode" | "exit command mode" | "turn off command mode"
        ),
        "ko" => matches!(
            spoken.replace(' ', "").as_str(),
            "명령모드종료" | "명령모드끄기"
        ),
        "ja" => matches!(
            spoken.replace(' ', "").as_str(),
            "コマンドモードを終了" | "コマンドモードをオフにして"
        ),
        _ => false,
    }
}

/// A short, unambiguous handoff can skip the remote command interpreter.
/// Keeping this exact avoids treating examples or negated phrases as actions.
pub(crate) fn is_start_voice_phrase(text: &str) -> bool {
    let spoken: String = text
        .chars()
        .filter(|ch| !ch.is_whitespace() && !"。.!！？?，,：:".contains(*ch))
        .flat_map(char::to_lowercase)
        .collect();
    matches!(
        spoken.as_str(),
        "打开语音识别"
            | "开启语音识别"
            | "开始语音识别"
            | "启动语音识别"
            | "打开语音输入"
            | "开启语音输入"
            | "开始语音输入"
            | "切换到语音输入"
            | "帮我打开语音识别"
            | "帮我开启语音识别"
            | "请打开语音识别"
            | "请开启语音识别"
            | "请帮我打开语音识别"
            | "请帮我开启语音识别"
            | "startvoiceinput"
            | "startvoicerecognition"
            | "startdictation"
            | "openvoiceinput"
            | "switchtovoiceinput"
            | "음성입력시작"
            | "음성인식시작"
            | "音声入力を開始"
            | "音声認識を開始"
    )
}

/// Keep a shortcut command together if ASR stops after its setting name or
/// midway through a key combination. A later utterance can complete it.
pub(crate) fn incomplete_shortcut_request(text: &str) -> bool {
    let spoken = text.trim().to_lowercase();
    let target = ["快捷键", "热键", "shortcut", "hotkey"]
        .iter()
        .filter_map(|word| spoken.find(word).map(|index| index + word.len()))
        .min();
    let Some(target_end) = target else {
        return false;
    };
    if ![
        "修改", "更改", "设置", "调整", "绑定", "改成", "change", "set",
    ]
    .iter()
    .any(|verb| spoken.contains(verb))
        || ["不要", "别修改", "别设置", "取消", "don't", "not "]
            .iter()
            .any(|negation| spoken.contains(negation))
        || ["然后", "接着", "并且", " and ", " then "]
            .iter()
            .any(|connector| spoken.contains(connector))
    {
        return false;
    }
    let value_start = ["为", "成", "到", " to ", " as "]
        .iter()
        .filter_map(|marker| spoken.rfind(marker).map(|index| index + marker.len()))
        .filter(|index| *index > target_end)
        .max();
    let Some(value_start) = value_start else {
        return true;
    };
    let value = spoken[value_start..]
        .trim_matches(|ch: char| ch.is_whitespace() || "。.!！？?，,：:".contains(ch))
        .replace([' ', '　'], "")
        .replace(['＋', '加'], "+");
    value.parse::<Shortcut>().is_err()
}

pub(crate) fn apply(config: &mut UiConfig, changes: &[SettingChange]) -> Result<String> {
    let mut proposed = config.clone();
    let summary = apply_checked(&mut proposed, changes)?;
    *config = proposed;
    Ok(summary)
}

fn apply_checked(config: &mut UiConfig, changes: &[SettingChange]) -> Result<String> {
    if changes.is_empty() || changes.len() > 4 {
        bail!("没有明确的设置指令，请说出设置项和期望的值");
    }
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    for change in changes {
        if !seen.insert(change.setting.as_str()) {
            bail!("同一个设置项不能在一条指令中重复修改");
        }
        let (name, current): (&str, Option<&mut bool>) = match change.setting.as_str() {
            "ai_translation" => ("AI 翻译", Some(&mut config.ai_translation)),
            "ai_auto_save" => ("AI 译文自动入库", Some(&mut config.ai_auto_save)),
            "continuous_input" => ("连续输入", Some(&mut config.continuous_input)),
            "ocr_auto_translate" => ("OCR 自动翻译", Some(&mut config.ocr_auto_translate)),
            "ocr_show_original" => ("OCR 显示原文", Some(&mut config.ocr_show_original)),
            "voice_keep_input" => ("持续语音输入", Some(&mut config.voice_keep_input)),
            "voice_hold_to_talk" => ("按住说话", Some(&mut config.voice_hold_to_talk)),
            "voice_auto_copy_first" => (
                "语音匹配自动选择第一条",
                Some(&mut config.voice_auto_copy_first),
            ),
            "voice_playback_filter" => ("过滤电脑播放声", Some(&mut config.voice_playback_filter)),
            "voice_owner_filter" => ("本人声音过滤", Some(&mut config.voice_owner_filter)),
            "voice_aion2_manual_paste" => {
                ("Aion2 兼容模式", Some(&mut config.voice_aion2_manual_paste))
            }
            "system_audio_monitor" => ("系统声音字幕", Some(&mut config.system_audio_monitor)),
            "system_audio_translate" => {
                ("系统声音字幕翻译", Some(&mut config.system_audio_translate))
            }
            "mic_tts_enabled" => ("自动麦克风发音", Some(&mut config.mic_tts_enabled)),
            "selected_text_context_menu" if SELECTED_TEXT_TRANSLATION_ACTIVE => (
                "选中文字后显示右键翻译入口",
                Some(&mut config.selected_text_context_menu),
            ),
            _ => ("", None),
        };
        if let Some(current) = current {
            let enabled = match &change.value {
                Value::Bool(enabled) => *enabled,
                Value::String(value) if value == "toggle" => !*current,
                _ => bail!("设置项 {} 需要开启或关闭", change.setting),
            };
            *current = enabled;
            if change.setting == "system_audio_translate" && enabled {
                config.system_audio_monitor = true;
            }
            names.push(format!(
                "已{}{}",
                if enabled { "开启" } else { "关闭" },
                name
            ));
            continue;
        }
        let numeric: Option<(&str, &mut f32, f64, f64)> = match change.setting.as_str() {
            "input.opacity" => Some(("输入框透明度", &mut config.input.opacity, 0.0, 1.0)),
            "input.width" => Some(("输入框宽度", &mut config.input.width, 80.0, 1200.0)),
            "input.height" => Some(("输入框高度", &mut config.input.height, 18.0, 200.0)),
            "input.font_size" => Some(("输入框字号", &mut config.input.font_size, 8.0, 72.0)),
            "input.corner_radius" => {
                Some(("输入框圆角", &mut config.input.corner_radius, 0.0, 100.0))
            }
            "results.opacity" => Some(("查询结果透明度", &mut config.results.opacity, 0.0, 1.0)),
            "results.min_width" => Some((
                "查询结果最小宽度",
                &mut config.results.min_width,
                80.0,
                1200.0,
            )),
            "results.max_width" => Some((
                "查询结果最大宽度",
                &mut config.results.max_width,
                200.0,
                1600.0,
            )),
            "results.min_row_height" => Some((
                "查询结果最小行高",
                &mut config.results.min_row_height,
                18.0,
                200.0,
            )),
            "results.max_height" => Some((
                "查询结果最大高度",
                &mut config.results.max_height,
                40.0,
                1200.0,
            )),
            "results.font_size" => Some(("查询结果字号", &mut config.results.font_size, 8.0, 72.0)),
            "results.corner_radius" => Some((
                "查询结果圆角",
                &mut config.results.corner_radius,
                0.0,
                100.0,
            )),
            "selected_opacity" => Some(("选中背景透明度", &mut config.selected_opacity, 0.0, 1.0)),
            "gap" => Some(("间距", &mut config.gap, 0.0, 40.0)),
            _ => None,
        };
        if let Some((name, field, min, max)) = numeric {
            let value = number(&change.value, min, max)?;
            *field = value as f32;
            names.push(format!("{name}已设为 {value}"));
            continue;
        }
        let color: Option<(&str, &mut String)> = match change.setting.as_str() {
            "input.background_color" => {
                Some(("输入框背景颜色", &mut config.input.background_color))
            }
            "results.background_color" => {
                Some(("查询结果背景颜色", &mut config.results.background_color))
            }
            "text_color" => Some(("文字颜色", &mut config.text_color)),
            "selected_color" => Some(("选中背景颜色", &mut config.selected_color)),
            "selected_text_color" => Some(("选中文字颜色", &mut config.selected_text_color)),
            _ => None,
        };
        if let Some((name, field)) = color {
            let value = change.value.as_str().unwrap_or_default();
            if value.len() != 7
                || !value.starts_with('#')
                || !value[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                bail!("颜色需要 #RRGGBB 格式");
            }
            *field = value.to_ascii_uppercase();
            names.push(format!("{name}已设为 {field}"));
            continue;
        }
        let hotkey: Option<(&str, &mut String)> = match change.setting.as_str() {
            "hotkeys.popup" => Some(("打开查询", &mut config.hotkeys.popup)),
            "hotkeys.settings" => Some(("打开设置", &mut config.hotkeys.settings)),
            "hotkeys.clear_composed" => Some(("清空组合内容", &mut config.hotkeys.clear_composed)),
            "hotkeys.ocr" => Some(("OCR 识别", &mut config.hotkeys.ocr)),
            "hotkeys.voice" => Some(("语音识别", &mut config.hotkeys.voice)),
            "hotkeys.command" => Some(("语音指令模式", &mut config.hotkeys.command)),
            "hotkeys.mic_tts" => Some(("自动麦克风发音", &mut config.hotkeys.mic_tts)),
            "hotkeys.system_audio" => {
                Some(("系统声音字幕与翻译", &mut config.hotkeys.system_audio))
            }
            "hotkeys.selected_translate" if SELECTED_TEXT_TRANSLATION_ACTIVE => {
                Some(("翻译选中文字", &mut config.hotkeys.selected_translate))
            }
            _ => None,
        };
        if let Some((name, field)) = hotkey {
            let value = change.value.as_str().unwrap_or_default();
            let spoken = value.replace([' ', '　'], "").replace(['＋', '加'], "+");
            let parsed = spoken.parse::<Shortcut>().map_err(|reason| {
                anyhow::anyhow!("快捷键格式无效：{reason}。请说出 Ctrl+Alt+O 这样的组合")
            })?;
            *field = match parsed {
                Shortcut::Keyboard(_) => spoken.to_ascii_lowercase(),
                Shortcut::Mouse(button) => button.to_string(),
            };
            names.push(format!("{name}快捷键已设为 {field}"));
            continue;
        }
        match change.setting.as_str() {
            "voice_silence_seconds" => {
                let value = number(&change.value, 0.2, 3.0)?;
                let value = (value * 10.0).round() / 10.0;
                config.voice_silence_seconds = value as f32;
                names.push(format!("语音停顿时间已设为 {value:.1} 秒"));
            }
            "command_silence_seconds" => {
                let value = number(&change.value, 0.3, 3.0)?;
                let value = (value * 10.0).round() / 10.0;
                config.command_silence_seconds = value as f32;
                names.push(format!("指令模式停顿时间已设为 {value:.1} 秒"));
            }
            "subtitle_font_size" => {
                let value = number(&change.value, 12.0, 48.0)?;
                config.subtitle_font_size = value as f32;
                names.push(format!("字幕字号已设为 {} px", value.round()));
            }
            "max_visible_results" => {
                let value = number(&change.value, 1.0, 100.0)?;
                if value.fract() != 0.0 {
                    bail!("最多展示匹配数需要整数");
                }
                config.results.max_visible_results = value as usize;
                names.push(format!("最多展示匹配数已设为 {value:.0}"));
            }
            "ai_politeness" => {
                let (level, label) = match change.value.as_str() {
                    Some("informal") => (PolitenessLevel::Informal, "非敬语"),
                    Some("conversational") => (PolitenessLevel::Conversational, "口语化敬语"),
                    Some("formal") => (PolitenessLevel::Formal, "官方化敬语"),
                    _ => bail!("敬语模式需要非敬语、口语化敬语或官方化敬语"),
                };
                config.ai_politeness = level;
                names.push(format!("敬语模式已设为{label}"));
            }
            "source_language"
            | "translation_language"
            | "voice_language"
            | "system_audio_language"
            | "ui_language" => {
                let value = change.value.as_str().unwrap_or_default();
                match change.setting.as_str() {
                    "source_language" | "voice_language" if value == "auto" => {}
                    "system_audio_language"
                        if matches!(value, "auto" | "target" | "cn" | "en" | "ja" | "ko") => {}
                    "ui_language" if matches!(value, "zh-CN" | "en" | "ko" | "ja") => {}
                    "source_language" | "translation_language" | "voice_language"
                        if translation_language::LANGUAGES
                            .iter()
                            .any(|language| language.code == value) => {}
                    _ => bail!("不支持的语言代码：{value}"),
                }
                match change.setting.as_str() {
                    "source_language" => config.source_language = value.into(),
                    "translation_language" => config.translation_language = value.into(),
                    "voice_language" => config.voice_language = value.into(),
                    "system_audio_language" => config.system_audio_language = value.into(),
                    "ui_language" => config.ui_language = value.into(),
                    _ => unreachable!(),
                }
                names.push(format!("语言设置已更新为 {value}"));
            }
            _ => bail!("暂不支持通过语音修改设置项：{}", change.setting),
        }
    }
    if config.results.max_width < config.results.min_width {
        bail!("查询结果最大宽度不能小于最小宽度");
    }
    let hotkeys = [
        &config.hotkeys.popup,
        &config.hotkeys.settings,
        &config.hotkeys.clear_composed,
        &config.hotkeys.ocr,
        &config.hotkeys.voice,
        &config.hotkeys.command,
        &config.hotkeys.mic_tts,
        &config.hotkeys.system_audio,
    ];
    let mut parsed = Vec::with_capacity(hotkeys.len());
    for hotkey in hotkeys {
        let shortcut = hotkey.parse::<Shortcut>().map_err(anyhow::Error::msg)?;
        if parsed.contains(&shortcut) {
            bail!("快捷键不能使用相同的组合");
        }
        parsed.push(shortcut);
    }
    Ok(names.join("；"))
}

fn number(value: &Value, min: f64, max: f64) -> Result<f64> {
    let Some(number) = value.as_f64() else {
        bail!("数值设置需要明确的数字");
    };
    if !number.is_finite() || !(min..=max).contains(&number) {
        bail!("设置数值超出允许范围：{min}–{max}");
    }
    Ok(number)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_voice_handoff_accepts_only_explicit_requests() {
        assert!(is_start_voice_phrase("打开语音识别。"));
        assert!(is_start_voice_phrase("Start voice input!"));
        assert!(!is_start_voice_phrase("不要打开语音识别"));
        assert!(!is_start_voice_phrase("如何打开语音识别？"));
        assert!(!is_start_voice_phrase("修改语音识别快捷键"));
    }

    #[test]
    fn immediate_actions_validate_payloads() {
        let change = |setting: &str, value: Value| SettingChange {
            setting: setting.into(),
            value,
            feedback: String::new(),
        };
        assert_eq!(
            parse_action(&change("action.open_ocr", Value::Null)).unwrap(),
            Some(CommandAction::OpenOcr)
        );
        assert_eq!(
            parse_action(&change("action.system_audio", Value::Bool(true))).unwrap(),
            Some(CommandAction::SystemAudio(Some(true)))
        );
        assert!(parse_action(&change("action.open_ocr", Value::Bool(true))).is_err());
        assert!(parse_action(&change("action.translate_text", Value::Null)).is_err());
        assert!(parse_action(&change("action.translate_selection", Value::Null)).is_err());
        assert!(parse_action(&change("action.run_program", Value::Null)).is_err());
    }

    #[test]
    fn accepts_known_setting_and_rejects_unknown() {
        let mut config = UiConfig::default();
        let changes = [SettingChange {
            setting: "ai_translation".into(),
            value: Value::Bool(true),
            ..Default::default()
        }];
        assert!(apply(&mut config, &changes).is_ok());
        assert!(config.ai_translation);
        let changes = [SettingChange {
            setting: "hotkeys.unknown".into(),
            value: Value::Bool(false),
            ..Default::default()
        }];
        assert!(apply(&mut config, &changes).is_err());
    }

    #[test]
    fn changes_ocr_hotkey_from_spoken_combo_and_rejects_conflict() {
        let mut config = UiConfig::default();
        apply(
            &mut config,
            &[SettingChange {
                setting: "hotkeys.ocr".into(),
                value: serde_json::json!("Ctrl 加 Alt 加 P"),
                ..Default::default()
            }],
        )
        .unwrap();
        assert_eq!(config.hotkeys.ocr, "ctrl+alt+p");
        let previous = config.hotkeys.ocr.clone();
        assert!(
            apply(
                &mut config,
                &[SettingChange {
                    setting: "hotkeys.ocr".into(),
                    value: serde_json::json!("ctrl+alt+s"),
                    ..Default::default()
                }]
            )
            .is_err()
        );
        assert_eq!(config.hotkeys.ocr, previous);

        apply(
            &mut config,
            &[SettingChange {
                setting: "hotkeys.ocr".into(),
                value: serde_json::json!("ctrl加alt加o"),
                ..Default::default()
            }],
        )
        .unwrap();
        assert_eq!(config.hotkeys.ocr, "ctrl+alt+o");
    }

    #[test]
    fn invalid_second_change_does_not_partially_apply_first() {
        let mut config = UiConfig::default();
        let changes = [
            SettingChange {
                setting: "ai_translation".into(),
                value: Value::Bool(true),
                ..Default::default()
            },
            SettingChange {
                setting: "voice_silence_seconds".into(),
                value: serde_json::json!(99),
                ..Default::default()
            },
        ];
        assert!(apply(&mut config, &changes).is_err());
        assert!(!config.ai_translation);
    }

    #[test]
    fn only_exact_exit_phrase_closes_mode() {
        assert!(is_exit_phrase("关闭指令模式。", "cn"));
        assert!(is_exit_phrase("stop command mode", "en"));
        assert!(!is_exit_phrase("不要关闭指令模式", "cn"));
    }

    #[test]
    fn waits_for_missing_shortcut_value_but_accepts_complete_combo() {
        assert!(incomplete_shortcut_request("修改 OCR 识别快捷键"));
        assert!(incomplete_shortcut_request(
            "修改 OCR 识别快捷键为 Ctrl 加 Alt"
        ));
        assert!(!incomplete_shortcut_request(
            "修改 OCR 识别快捷键为 Ctrl 加 Alt 加 O"
        ));
        assert!(!incomplete_shortcut_request("不要修改 OCR 识别快捷键"));
        assert!(!incomplete_shortcut_request("开启 AI 翻译"));
    }
}
