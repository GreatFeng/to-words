//! System playback recognition and optional subtitle translation.

use super::*;

struct SubtitleTranslation {
    generation: u64,
    utterance_id: u64,
    original: String,
    target: String,
    result: Result<String, String>,
}

#[tauri::command]
pub(super) fn get_system_audio_display(state: State<'_, WebState>) -> Result<VoiceDisplay, String> {
    state
        .system_audio_display
        .lock()
        .map(|value| value.clone())
        .map_err(error)
}

pub(super) fn toggle_system_audio(app: &AppHandle) {
    use tauri::Emitter;
    let state = app.state::<WebState>();
    let changed = (|| -> Result<bool, String> {
        let mut config = state.config.lock().map_err(error)?;
        let enabled = !(config.system_audio_monitor && config.system_audio_translate);
        let mut next = config.clone();
        next.system_audio_monitor = enabled;
        next.system_audio_translate = enabled;
        save_ui_config(&next).map_err(error)?;
        *config = next;
        Ok(enabled)
    })();
    match changed {
        Ok(enabled) => {
            let status = if enabled {
                "系统声音字幕与翻译已开启"
            } else {
                "系统声音字幕与翻译已关闭"
            };
            let _ = app.emit_to("settings", "system-audio-toggle", enabled);
            let _ = app.emit_to("settings", "system-audio-status", status);
        }
        Err(reason) => {
            eprintln!("切换系统声音字幕失败：{reason}");
            let _ = app.emit_to("settings", "system-audio-status", reason);
        }
    }
}

fn position_system_audio(app: &AppHandle) {
    let Some(window) = app.get_webview_window("system_audio") else {
        return;
    };
    let saved = app
        .state::<WebState>()
        .config
        .lock()
        .ok()
        .and_then(|config| config.subtitle_bounds);
    let (x, y, width, height) = if let Some(saved) = saved {
        (saved.x, saved.y, saved.width, saved.height)
    } else {
        let height = 115_u32;
        let Some((x, y, width)) = crate::platform::window_monitor::subtitle_bounds(1020, height)
        else {
            return;
        };
        (x, y, width, height)
    };
    let _ = window.set_size(tauri::Size::Physical(tauri::PhysicalSize::new(
        width, height,
    )));
    let _ = window.set_position(tauri::Position::Physical(tauri::PhysicalPosition::new(
        x, y,
    )));
}

fn show_subtitle(app: &AppHandle, display: VoiceDisplay) -> bool {
    use tauri::Emitter;
    if let Ok(mut current) = app.state::<WebState>().system_audio_display.lock() {
        *current = display.clone();
    }
    if let Some(window) = app.get_webview_window("system_audio") {
        if !window.is_visible().unwrap_or(false) {
            position_system_audio(app);
        }
        let shown = window.show();
        let sent = app.emit_to("system_audio", "system-audio-display", display);
        if let Err(reason) = &shown {
            eprintln!("系统声音字幕窗口无法显示：{reason}");
        }
        if let Err(reason) = &sent {
            eprintln!("系统声音字幕更新失败：{reason}");
        }
        shown.is_ok() && sent.is_ok()
    } else {
        false
    }
}

#[tauri::command]
pub(super) fn save_subtitle_bounds(
    bounds: crate::web_config::SubtitleBounds,
    state: State<'_, WebState>,
) -> Result<(), String> {
    let mut config = state.config.lock().map_err(error)?;
    config.subtitle_bounds = Some(bounds);
    *config = config.clone().normalized();
    save_ui_config(&config).map_err(error)
}

fn recognition_language(config: &UiConfig) -> &'static str {
    let selected = if config.system_audio_language == "target" {
        config.translation_language.as_str()
    } else {
        config.system_audio_language.as_str()
    };
    match selected {
        "cn" | "zh_tw" => "cn",
        "en" => "en",
        "ja" => "ja",
        "ko" => "ko",
        _ => "auto",
    }
}

fn hide_subtitle(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("system_audio") {
        let _ = window.hide();
    }
}

pub(super) fn start_system_audio_worker(app: AppHandle) {
    std::thread::spawn(move || {
        let (translation_tx, translation_rx) = mpsc::channel::<SubtitleTranslation>();
        let mut session: Option<VoiceSession> = None;
        let mut active_settings: Option<(String, u32, String, String)> = None;
        let mut generation = 0_u64;
        let mut latest_utterance = 0_u64;
        let mut latest_final_id = 0_u64;
        let mut latest_partial_id = 0_u64;
        let mut caption_seq = 0_u64;
        let mut latest_visible_seq = 0_u64;
        let mut browser_was_active = false;
        let mut last_browser_text = String::new();
        let mut pending_browser_translation: Option<(String, u64, Instant)> = None;
        let mut empty_utterances = 0_u32;
        let mut hide_at: Option<Instant> = None;
        let mut retry_at = Instant::now();
        let mut was_enabled = false;
        let mut recent_text: Option<(String, Instant)> = None;
        let mut awaiting_audio_since: Option<Instant> = None;
        loop {
            let config = match app.state::<WebState>().config.lock() {
                Ok(config) => config.clone(),
                Err(_) => break,
            };
            let (track_active, browser_text) = app
                .state::<WebState>()
                .browser_caption
                .lock()
                .map(|caption| (caption.active(), caption.text.clone()))
                .unwrap_or_default();
            let browser_active = config.system_audio_monitor && track_active;
            let settings = (
                config.voice_backend.clone(),
                config.voice_silence_seconds.to_bits(),
                config.system_audio_device.clone(),
                recognition_language(&config).to_owned(),
            );
            if !config.system_audio_monitor {
                if was_enabled {
                    generation = generation.wrapping_add(1);
                    hide_subtitle(&app);
                    hide_at = None;
                    awaiting_audio_since = None;
                }
                if let Some(active) = &mut session {
                    if !active.stopping {
                        active.stop();
                    }
                }
                was_enabled = false;
            } else {
                if browser_active {
                    if let Some(active) = &mut session
                        && !active.stopping
                    {
                        active.stop();
                    }
                }
                if active_settings.as_ref() != Some(&settings) {
                    if let Some(active) = &mut session {
                        if !active.stopping {
                            active.stop();
                        }
                    }
                    active_settings = Some(settings);
                }
                was_enabled = true;
                if !browser_active && session.is_none() && Instant::now() >= retry_at {
                    match voice::start_playback(
                        &config.voice_backend,
                        config.voice_silence_seconds,
                        &config.system_audio_device,
                        recognition_language(&config),
                    ) {
                        Ok(started) => {
                            generation = generation.wrapping_add(1);
                            latest_utterance = 0;
                            latest_final_id = 0;
                            latest_partial_id = 0;
                            latest_visible_seq = 0;
                            empty_utterances = 0;
                            session = Some(started);
                            awaiting_audio_since = Some(Instant::now());
                            show_subtitle(
                                &app,
                                VoiceDisplay {
                                    status: "正在监听系统播放声音…".into(),
                                    ..Default::default()
                                },
                            );
                            hide_at = Some(Instant::now() + Duration::from_secs(3));
                        }
                        Err(reason) => {
                            let status = format!("系统声音监听无法启动：{reason:#}");
                            eprintln!("{status}");
                            show_subtitle(
                                &app,
                                VoiceDisplay {
                                    status,
                                    ..Default::default()
                                },
                            );
                            hide_at = Some(Instant::now() + Duration::from_secs(4));
                            retry_at = Instant::now() + Duration::from_secs(8);
                        }
                    }
                }
            }

            if browser_active != browser_was_active {
                generation = generation.wrapping_add(1);
                latest_visible_seq = 0;
                last_browser_text.clear();
                pending_browser_translation = None;
                browser_was_active = browser_active;
            }
            if browser_active && !browser_text.is_empty() && browser_text != last_browser_text {
                last_browser_text = browser_text.clone();
                caption_seq = caption_seq.wrapping_add(1);
                latest_visible_seq = caption_seq;
                show_subtitle(
                    &app,
                    VoiceDisplay {
                        original: browser_text.clone(),
                        ..Default::default()
                    },
                );
                hide_at = Some(Instant::now() + Duration::from_secs(5));
                pending_browser_translation =
                    Some((browser_text.clone(), latest_visible_seq, Instant::now()));
            }
            if browser_active && !browser_text.is_empty() {
                hide_at = Some(Instant::now() + Duration::from_secs(5));
            } else if browser_active && browser_text.is_empty() && !last_browser_text.is_empty() {
                last_browser_text.clear();
                caption_seq = caption_seq.wrapping_add(1);
                latest_visible_seq = caption_seq;
                pending_browser_translation = None;
                hide_subtitle(&app);
                hide_at = None;
            }
            if !config.system_audio_translate {
                pending_browser_translation = None;
            }
            if pending_browser_translation
                .as_ref()
                .is_some_and(|(_, _, since)| since.elapsed() >= Duration::from_millis(350))
            {
                let (original, id, _) = pending_browser_translation.take().unwrap();
                let target = if config.source_language == "auto" {
                    "cn".to_owned()
                } else {
                    config.source_language.clone()
                };
                if browser_active
                    && browser_text == original
                    && translation_language::detect_source_language(&original)
                        != Some(target.as_str())
                {
                    match credentials::load_key() {
                        Ok(Some(key)) if !key.trim().is_empty() => {
                            let sender = translation_tx.clone();
                            let politeness = config.ai_politeness;
                            let request_generation = generation;
                            std::thread::spawn(move || {
                                let result = ai::translate_subtitle_text(
                                    &original, &target, &key, politeness,
                                )
                                .map_err(error);
                                let _ = sender.send(SubtitleTranslation {
                                    generation: request_generation,
                                    utterance_id: id,
                                    original,
                                    target,
                                    result,
                                });
                            });
                        }
                        _ => {
                            show_subtitle(
                                &app,
                                VoiceDisplay {
                                    original,
                                    status: "请先设置 DeepSeek API Key".into(),
                                    ..Default::default()
                                },
                            );
                        }
                    }
                }
            }

            let events = session
                .as_ref()
                .map(|active| active.receiver.try_iter().collect::<Vec<_>>())
                .unwrap_or_default();
            for event in events {
                match event {
                    VoiceEvent::Partial(id, text)
                        if config.system_audio_monitor
                            && !browser_active
                            && id > latest_final_id =>
                    {
                        let original = text.trim().to_owned();
                        if !original.is_empty() && !voice_word_cache::is_obviously_bad(&original) {
                            latest_partial_id = id;
                            show_subtitle(
                                &app,
                                VoiceDisplay {
                                    original,
                                    ..Default::default()
                                },
                            );
                            hide_at = Some(Instant::now() + Duration::from_secs(5));
                        }
                    }
                    VoiceEvent::SpeechStarted(_) if config.system_audio_monitor => {
                        if browser_active {
                            continue;
                        }
                        awaiting_audio_since = None;
                        if latest_utterance == 0 && empty_utterances == 0 {
                            show_subtitle(
                                &app,
                                VoiceDisplay {
                                    status: "检测到播放声音，正在生成字幕…".into(),
                                    ..Default::default()
                                },
                            );
                            hide_at = Some(Instant::now() + Duration::from_secs(5));
                        }
                    }
                    VoiceEvent::Recognizing(id) if config.system_audio_monitor => {
                        if browser_active {
                            continue;
                        }
                        if latest_partial_id == id {
                            continue;
                        }
                        if latest_utterance == 0 && empty_utterances == 0 {
                            show_subtitle(
                                &app,
                                VoiceDisplay {
                                    status: "正在识别系统播放声音…".into(),
                                    ..Default::default()
                                },
                            );
                            hide_at = Some(Instant::now() + Duration::from_secs(5));
                        }
                    }
                    VoiceEvent::Utterance(id, Ok(text)) if config.system_audio_monitor => {
                        latest_final_id = id;
                        if browser_active {
                            continue;
                        }
                        let original = text.trim().to_owned();
                        if original.is_empty() || voice_word_cache::is_obviously_bad(&original) {
                            empty_utterances = empty_utterances.saturating_add(1);
                            if let Some(active) = &session {
                                active
                                    .trace
                                    .record(id, "subtitle_text", None, "empty_or_bad");
                            }
                            if latest_utterance == 0 {
                                show_subtitle(
                                    &app,
                                    VoiceDisplay {
                                        status: "已收到声音，但尚未识别出可显示的文字".into(),
                                        ..Default::default()
                                    },
                                );
                                hide_at = Some(Instant::now() + Duration::from_secs(4));
                            } else if latest_partial_id == id {
                                hide_subtitle(&app);
                                hide_at = None;
                            }
                            continue;
                        }
                        empty_utterances = 0;
                        if recent_text.as_ref().is_some_and(|(last, when)| {
                            last == &original && when.elapsed() < Duration::from_secs(2)
                        }) {
                            if let Some(active) = &session {
                                active.trace.record(id, "subtitle_text", None, "duplicate");
                            }
                            if latest_partial_id == id {
                                show_subtitle(
                                    &app,
                                    VoiceDisplay {
                                        original,
                                        ..Default::default()
                                    },
                                );
                                hide_at = Some(Instant::now() + Duration::from_secs(5));
                            }
                            continue;
                        }
                        latest_utterance = id;
                        caption_seq = caption_seq.wrapping_add(1);
                        latest_visible_seq = caption_seq;
                        recent_text = Some((original.clone(), Instant::now()));
                        let displayed = show_subtitle(
                            &app,
                            VoiceDisplay {
                                original: original.clone(),
                                ..Default::default()
                            },
                        );
                        if let Some(active) = &session {
                            active.trace.record(
                                id,
                                "subtitle_display",
                                None,
                                if displayed { "ok" } else { "error" },
                            );
                        }
                        hide_at = Some(Instant::now() + Duration::from_secs(5));
                        if !config.system_audio_translate {
                            continue;
                        }
                        let target = if config.source_language == "auto" {
                            "cn".to_owned()
                        } else {
                            config.source_language.clone()
                        };
                        if recognition_language(&config) == target
                            || translation_language::detect_source_language(&original)
                                == Some(target.as_str())
                        {
                            continue;
                        }
                        let key = match credentials::load_key() {
                            Ok(Some(key)) if !key.trim().is_empty() => key,
                            _ => {
                                show_subtitle(
                                    &app,
                                    VoiceDisplay {
                                        original,
                                        result: String::new(),
                                        status: "请先设置 DeepSeek API Key".into(),
                                    },
                                );
                                continue;
                            }
                        };
                        let sender = translation_tx.clone();
                        let politeness = config.ai_politeness;
                        let request_generation = generation;
                        let request_id = latest_visible_seq;
                        // Keep the recognized text visible until the network result arrives.
                        hide_at = Some(Instant::now() + Duration::from_secs(35));
                        std::thread::spawn(move || {
                            let result =
                                ai::translate_subtitle_text(&original, &target, &key, politeness)
                                    .map_err(error);
                            let _ = sender.send(SubtitleTranslation {
                                generation: request_generation,
                                utterance_id: request_id,
                                original,
                                target,
                                result,
                            });
                        });
                    }
                    VoiceEvent::Utterance(_, Err(reason)) if config.system_audio_monitor => {
                        if browser_active {
                            continue;
                        }
                        eprintln!("系统声音识别失败：{reason}");
                        show_subtitle(
                            &app,
                            VoiceDisplay {
                                status: format!("系统声音识别失败：{reason}"),
                                ..Default::default()
                            },
                        );
                        hide_at = Some(Instant::now() + Duration::from_secs(3));
                    }
                    VoiceEvent::Stopped(reason) => {
                        session = None;
                        awaiting_audio_since = None;
                        if let Some(reason) = reason {
                            if config.system_audio_monitor && !browser_active {
                                let status = format!("系统声音监听已中断：{reason}");
                                eprintln!("{status}");
                                show_subtitle(
                                    &app,
                                    VoiceDisplay {
                                        status,
                                        ..Default::default()
                                    },
                                );
                                hide_at = Some(Instant::now() + Duration::from_secs(4));
                                retry_at = Instant::now() + Duration::from_secs(8);
                            }
                        } else {
                            retry_at = Instant::now();
                        }
                    }
                    _ => {}
                }
            }
            if !browser_active
                && awaiting_audio_since
                    .is_some_and(|since| since.elapsed() >= Duration::from_secs(10))
            {
                awaiting_audio_since = None;
                show_subtitle(
                    &app,
                    VoiceDisplay {
                        status: "未检测到播放声音，请在设置中选择视频使用的播放设备".into(),
                        ..Default::default()
                    },
                );
                hide_at = Some(Instant::now() + Duration::from_secs(6));
            }
            for translated in translation_rx.try_iter() {
                if !config.system_audio_monitor
                    || !config.system_audio_translate
                    || translated.generation != generation
                    || translated.utterance_id != latest_visible_seq
                    || translated.target
                        != if config.source_language == "auto" {
                            "cn"
                        } else {
                            &config.source_language
                        }
                {
                    continue;
                }
                let (result, status) = match translated.result {
                    Ok(text) => (text, String::new()),
                    Err(reason) => (String::new(), format!("字幕翻译失败：{reason}")),
                };
                show_subtitle(
                    &app,
                    VoiceDisplay {
                        original: translated.original,
                        result,
                        status,
                    },
                );
                hide_at = Some(Instant::now() + Duration::from_secs(6));
            }
            if hide_at.is_some_and(|deadline| Instant::now() >= deadline) {
                hide_subtitle(&app);
                hide_at = None;
            }
            std::thread::sleep(Duration::from_millis(80));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{UiConfig, recognition_language};

    #[test]
    fn subtitle_language_follows_target_unless_overridden() {
        let mut config = UiConfig::default();
        config.translation_language = "ko".into();
        config.system_audio_language = "target".into();
        assert_eq!(recognition_language(&config), "ko");
        config.system_audio_language = "ja".into();
        assert_eq!(recognition_language(&config), "ja");
        config.system_audio_language = "auto".into();
        assert_eq!(recognition_language(&config), "auto");
    }
}
