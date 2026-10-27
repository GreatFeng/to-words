//! 语音输入、词库缓存审核与语音浮层。

use super::*;

#[tauri::command]
pub(super) async fn review_voice_cache_now(state: State<'_, WebState>) -> Result<(), String> {
    let sender = state
        .voice_cache
        .lock()
        .map_err(error)?
        .clone()
        .ok_or("语音词库审核服务尚未启动")?;
    let (reply, receiver) = mpsc::channel();
    sender.send(CacheCommand::Review(reply)).map_err(error)?;
    tauri::async_runtime::spawn_blocking(move || receiver.recv().map_err(error))
        .await
        .map_err(error)??
}

pub(super) fn start_cache_worker(app: AppHandle, receiver: Receiver<CacheCommand>) {
    std::thread::spawn(move || {
        let mut cache = match VoiceWordCache::load(storage::database_path()) {
            Ok(cache) => cache,
            Err(reason) => {
                eprintln!("无法加载语音词库缓存：{reason:#}");
                return;
            }
        };
        let mut last_review = if cache.pending_len() > 0 {
            Instant::now() - voice_word_cache::REVIEW_INTERVAL
        } else {
            Instant::now()
        };
        loop {
            let mut reply = None;
            match receiver.recv_timeout(Duration::from_secs(30)) {
                Ok(CacheCommand::Candidate(candidate)) => {
                    if let Err(reason) = cache.enqueue(candidate) {
                        eprintln!("语音词条暂存失败：{reason:#}");
                    }
                }
                Ok(CacheCommand::Review(sender)) => reply = Some(sender),
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if let Err(reason) = cache.expire_uncertain() {
                eprintln!("语音缓存清理失败：{reason:#}");
            }
            let should_review = reply.is_some()
                || cache.reviewable_len() >= voice_word_cache::REVIEW_THRESHOLD
                || (cache.pending_len() > 0
                    && last_review.elapsed() >= voice_word_cache::REVIEW_INTERVAL);
            if !should_review {
                continue;
            }
            last_review = Instant::now();
            let result = (|| -> Result<(), String> {
                if cache.reviewable_len() == 0 {
                    return Ok(());
                }
                let key = credentials::load_key()
                    .map_err(error)?
                    .ok_or("请先设置 DeepSeek API Key 才能审核语音缓存")?;
                while cache.reviewable_len() > 0 {
                    let before = cache.reviewable_len();
                    let saved = cache.review_once(&key).map_err(error)?;
                    if !saved.is_empty() {
                        app.state::<WebState>()
                            .search
                            .lock()
                            .map_err(error)?
                            .library
                            .clear();
                    }
                    if cache.reviewable_len() >= before {
                        break;
                    }
                }
                Ok(())
            })();
            if let Some(reply) = reply {
                let _ = reply.send(result);
            } else if let Err(reason) = result {
                eprintln!("语音缓存审核暂未完成：{reason}");
            }
        }
    });
}

#[tauri::command]
pub(super) fn get_voice_display(state: State<'_, WebState>) -> Result<VoiceDisplay, String> {
    state
        .voice_display
        .lock()
        .map(|value| value.clone())
        .map_err(error)
}

#[tauri::command]
pub(super) fn toggle_voice(state: State<'_, WebState>) -> Result<(), String> {
    state
        .voice_control
        .lock()
        .map_err(error)?
        .as_ref()
        .ok_or("语音服务尚未启动")?
        .send(VoiceControl::Toggle)
        .map_err(error)
}

pub(super) fn start_command_mode(app: &AppHandle) -> Result<(), String> {
    position_voice(app);
    app.state::<WebState>()
        .voice_control
        .lock()
        .map_err(error)?
        .as_ref()
        .ok_or("语音服务尚未启动")?
        .send(VoiceControl::Command)
        .map_err(error)
}

fn begin_command_recording(app: &AppHandle) -> Result<VoiceSession, String> {
    if !credentials::load_key()
        .map_err(error)?
        .is_some_and(|key| !key.trim().is_empty())
    {
        return Err("请先在设置中填写 DeepSeek API Key".into());
    }
    let config = app
        .state::<WebState>()
        .config
        .lock()
        .map_err(error)?
        .clone();
    position_voice(app);
    show_voice(
        app,
        VoiceDisplay {
            status: "指令模式：正在启动语音识别…".into(),
            ..Default::default()
        },
    );
    voice::start(
        true,
        false,
        "auto",
        &config.voice_backend,
        None,
        config.command_silence_seconds,
        config.voice_owner_filter,
        config.voice_playback_filter,
    )
    .map_err(error)
}

fn begin_normal_recording(
    app: &AppHandle,
    started_by_command: bool,
) -> Result<(VoiceSession, Option<Instant>), String> {
    let config = app
        .state::<WebState>()
        .config
        .lock()
        .map_err(error)?
        .clone();
    let hold_to_talk = config.voice_hold_to_talk && !started_by_command;
    position_voice(app);
    show_voice(
        app,
        VoiceDisplay {
            status: "正在启动语音识别…".into(),
            ..Default::default()
        },
    );
    let started = voice::start_with_idle_timeout(
        config.voice_keep_input || hold_to_talk,
        hold_to_talk,
        voice::effective_voice_language(
            &config.voice_language,
            &config.source_language,
            &config.translation_language,
        ),
        voice::effective_voice_backend(
            &config.voice_backend,
            &config.source_language,
            &config.translation_language,
        ),
        translation_language::same_language_pair(
            &config.source_language,
            &config.translation_language,
        ),
        config.voice_silence_seconds,
        config.voice_owner_filter,
        config.voice_playback_filter,
        Duration::from_secs(if started_by_command { 45 } else { 15 }),
    )
    .map_err(error)?;
    started.trace.record(0, "voice_session_ready", None, "ok");
    started.trace.record(
        0,
        "mic_tts_enabled",
        None,
        if config.mic_tts_enabled {
            "enabled"
        } else {
            "disabled"
        },
    );
    show_voice(
        app,
        VoiceDisplay {
            status: if started_by_command {
                "普通语音输入：正在聆听…"
            } else {
                "正在聆听…"
            }
            .into(),
            ..Default::default()
        },
    );
    Ok((started, hold_to_talk.then(Instant::now)))
}

fn command_exit_feedback(language: &str) -> &'static str {
    match language {
        "en" => "Command mode is off",
        "ko" => "명령 모드를 종료했어요",
        "ja" => "コマンドモードを終了しました",
        "zh_tw" => "已關閉指令模式",
        _ => "指令模式已关闭",
    }
}

fn command_voice_start_feedback(language: &str) -> &'static str {
    match language {
        "en" => "Switching to voice input",
        "ko" => "음성 입력으로 전환합니다",
        "ja" => "音声入力に切り替えます",
        "zh_tw" => "正在切換到語音輸入",
        _ => "正在切换到语音输入",
    }
}

fn command_success_feedback(
    change: &settings_command::SettingChange,
    language: &str,
    summary: &str,
) -> String {
    if matches!(language, "cn" | "auto") {
        summary.to_owned()
    } else if !change.feedback.trim().is_empty() {
        change.feedback.trim().to_owned()
    } else {
        summary.to_owned()
    }
}

#[cfg(windows)]
pub(super) fn voice_hotkey_is_down(binding: &str) -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    let key = binding
        .rsplit('+')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let virtual_key = match key.as_str() {
        "enter" | "return" => 0x0d,
        "space" => 0x20,
        "tab" => 0x09,
        "backspace" => 0x08,
        "escape" | "esc" => 0x1b,
        "mousex1" | "mouse4" => 0x05,
        "mousex2" | "mouse5" => 0x06,
        _ if key.len() == 1 && key.is_ascii() => key.as_bytes()[0].to_ascii_uppercase() as i32,
        _ if key.starts_with('f') => key[1..]
            .parse::<i32>()
            .ok()
            .filter(|number| (1..=24).contains(number))
            .map_or(0, |number| 0x6f + number),
        _ => 0,
    };
    virtual_key != 0 && unsafe { GetAsyncKeyState(virtual_key) as u16 & 0x8000 != 0 }
}

#[cfg(not(windows))]
pub(super) fn voice_hotkey_is_down(_binding: &str) -> bool {
    false
}

#[tauri::command]
pub(super) fn voice_registered() -> bool {
    voice_identity::is_registered()
}

#[tauri::command]
pub(super) fn delete_voice_profile(state: State<'_, WebState>) -> Result<(), String> {
    voice_identity::clear_profile().map_err(error)?;
    let mut config = state.config.lock().map_err(error)?;
    config.voice_owner_filter = false;
    save_ui_config(&config).map_err(error)
}

#[tauri::command]
pub(super) async fn register_voice_profile(
    app: AppHandle,
    state: State<'_, WebState>,
) -> Result<(), String> {
    use tauri::Emitter;
    let playback_filter = state.config.lock().map_err(error)?.voice_playback_filter;
    let receiver = voice_identity::start_enrollment(playback_filter).map_err(error)?;
    tauri::async_runtime::spawn_blocking(move || {
        for event in receiver {
            match event {
                EnrollmentEvent::Captured(count) => {
                    let _ = app.emit_to("settings", "voice-enrollment", format!("{count}/3"));
                }
                EnrollmentEvent::Finished(result) => {
                    let status = if result.is_ok() {
                        "声纹注册完成".to_string()
                    } else {
                        format!("声纹注册失败：{}", result.as_ref().err().unwrap())
                    };
                    let _ = app.emit_to("settings", "voice-enrollment", status);
                    return result.map_err(error);
                }
            }
        }
        Err("声纹注册任务意外结束".to_string())
    })
    .await
    .map_err(error)?
}

pub(super) fn show_voice(app: &AppHandle, display: VoiceDisplay) {
    use tauri::Emitter;
    if let Ok(mut current) = app.state::<WebState>().voice_display.lock() {
        *current = display.clone();
    }
    if let Some(window) = app.get_webview_window("voice") {
        let _ = window.show();
        let _ = app.emit_to("voice", "voice-display", display);
    }
}

pub(super) fn hide_voice(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("voice") {
        let _ = window.hide();
    }
}

pub(super) fn position_voice(app: &AppHandle) {
    let Ok(monitor) = ocr::monitor_under_cursor() else {
        return;
    };
    let Some(window) = app.get_webview_window("voice") else {
        return;
    };
    let width = monitor.width.min(900);
    let height = 120_u32;
    let x = monitor.x + (monitor.width as i32 - width as i32) / 2;
    let y = monitor.y + monitor.height as i32 - height as i32 - 120;
    let _ = window.set_size(tauri::Size::Physical(tauri::PhysicalSize::new(
        width, height,
    )));
    let _ = window.set_position(tauri::Position::Physical(tauri::PhysicalPosition::new(
        x, y,
    )));
}

pub(super) fn type_voice_result(result: &str, config: &UiConfig) -> String {
    match VoiceInputTarget::capture().map_or(Ok(None), |target| {
        target.type_if_active(
            result,
            config.voice_aion2_manual_paste,
            config.voice_hold_to_talk,
        )
    }) {
        Ok(Some(_)) => String::new(),
        Ok(None) => "当前文本光标不可用，未输入结果".to_string(),
        Err(reason) => format!("输入结果失败：{reason:#}"),
    }
}

pub(super) fn output_voice(app: &AppHandle, original: String, result: String, config: &UiConfig) {
    let mut status = type_voice_result(&result, config);
    if let Err(reason) = queue_tts(app.state::<WebState>().inner(), config, &result) {
        if !status.is_empty() {
            status.push_str("；");
        }
        status.push_str(&reason);
    }
    show_voice(
        app,
        VoiceDisplay {
            original,
            result,
            status,
        },
    );
}

pub(super) fn process_voice_text(
    app: &AppHandle,
    lookup: &mut VoiceLookup,
    config: &UiConfig,
    session_id: u64,
    utterance_id: u64,
    text: String,
    previous: Option<String>,
    translations: &Sender<VoiceTranslation>,
) -> bool {
    let query = text.trim();
    if voice_word_cache::is_obviously_bad(query) {
        show_voice(
            app,
            VoiceDisplay {
                original: query.into(),
                result: String::new(),
                status: "没有识别到清晰的语音".into(),
            },
        );
        return false;
    }
    if let Some(language) = translation_language::same_language_pair(
        &config.source_language,
        &config.translation_language,
    ) {
        if !translation_language::accepts_same_language_text(query, language) {
            show_voice(
                app,
                VoiceDisplay {
                    original: query.into(),
                    result: String::new(),
                    status: format!(
                        "识别结果不是所选的{}，已忽略",
                        translation_language::language_label(language)
                    ),
                },
            );
            return false;
        }
        output_voice(app, query.into(), query.into(), config);
        return false;
    }
    let source = if config.source_language == "auto" {
        translation_language::detect_source_language(query)
            .unwrap_or("cn")
            .to_owned()
    } else {
        config.source_language.clone()
    };
    let library = library_name(&source, &config.translation_language);
    if lookup.library != library {
        match storage::load_search_rows(&library) {
            Ok(words) => {
                lookup.library = library;
                lookup.index = WordIndex::from_rows(words);
            }
            Err(reason) => {
                show_voice(
                    app,
                    VoiceDisplay {
                        original: query.into(),
                        result: String::new(),
                        status: format!("读取词库失败：{reason:#}"),
                    },
                );
                return false;
            }
        }
    }
    let mut hits = lookup.index.search(query, "", &[]);
    let mut matched = query;
    if hits.is_empty() {
        let cleaned =
            query.trim_matches(|ch: char| ch.is_whitespace() || "。，！？、.,!?\"'“”".contains(ch));
        if !cleaned.is_empty() && cleaned != query {
            hits = lookup.index.search(cleaned, "", &[]);
            matched = cleaned;
        }
    }
    if let Some(first) = hits.first() {
        if hits.len() > 1 && !config.voice_auto_copy_first {
            if let Ok(mut pending) = app.state::<WebState>().pending_query.lock() {
                *pending = Some(matched.to_owned());
            }
            open_query(app);
            show_voice(
                app,
                VoiceDisplay {
                    original: query.into(),
                    result: String::new(),
                    status: "找到多条结果，请在查询框选择".into(),
                },
            );
            return false;
        }
        if let Some(entry) = lookup.index.get(first.entry_id) {
            let result = if entry.key.contains(matched) && !entry.value.contains(matched) {
                entry.value.clone()
            } else {
                entry.key.clone()
            };
            output_voice(app, query.into(), result, config);
            return false;
        }
    }
    if !config.ai_translation || config.translation_language.is_empty() {
        show_voice(
            app,
            VoiceDisplay {
                original: query.into(),
                result: String::new(),
                status: "词库无匹配，AI 翻译未启用".into(),
            },
        );
        return false;
    }
    let key = match credentials::load_key() {
        Ok(Some(key)) if !key.trim().is_empty() => key,
        _ => {
            show_voice(
                app,
                VoiceDisplay {
                    original: query.into(),
                    result: String::new(),
                    status: "词库无匹配，请设置 DeepSeek API Key".into(),
                },
            );
            return false;
        }
    };
    show_voice(
        app,
        VoiceDisplay {
            original: query.into(),
            result: String::new(),
            status: "本地词库无匹配，正在翻译…".into(),
        },
    );
    let target = config.translation_language.clone();
    let reverse = if config.source_language == "auto" {
        "cn".to_string()
    } else {
        source.clone()
    };
    let politeness = config.ai_politeness;
    let recognized = query.to_owned();
    let sender = translations.clone();
    std::thread::spawn(move || {
        let result =
            ai::translate_voice(recognized.clone(), source, target, reverse, key, politeness)
                .map_err(error);
        let _ = sender.send(VoiceTranslation {
            session_id,
            utterance_id,
            recognized,
            previous,
            result,
        });
    });
    true
}

pub(super) fn start_voice_worker(app: AppHandle, controls: Receiver<VoiceControl>) {
    std::thread::spawn(move || {
        let (translation_tx, translation_rx) = mpsc::channel::<VoiceTranslation>();
        let (command_tx, command_rx) = mpsc::channel::<CommandInterpretation>();
        let mut session: Option<VoiceSession> = None;
        let mut command_mode = false;
        let mut pending_command = false;
        let mut pending_normal_voice = false;
        let mut resume_command_after_voice = false;
        let mut command_ai_pending = false;
        let mut command_utterances = VecDeque::<String>::new();
        let mut command_fragment: Option<(String, Instant)> = None;
        let mut command_queue = VecDeque::<QueuedSettingCommand>::new();
        let mut command_feedback_until: Option<Instant> = None;
        let mut suppress_voice_events = false;
        let mut session_id = 0_u64;
        let mut latest_utterance = 0_u64;
        let mut utterance_starts = HashMap::<u64, Instant>::new();
        let mut ai_pending = 0_usize;
        let mut lookup = VoiceLookup::default();
        let mut context = VecDeque::<String>::new();
        let mut hide_at: Option<Instant> = None;
        let mut hold_started: Option<Instant> = None;
        loop {
            if let Ok(control) = controls.try_recv() {
                if !matches!(control, VoiceControl::Release) {
                    pending_normal_voice = false;
                }
                if matches!(control, VoiceControl::Command) {
                    if command_mode || resume_command_after_voice {
                        if let Some(active) = &mut session {
                            active.stop();
                            suppress_voice_events = true;
                        }
                        command_mode = false;
                        resume_command_after_voice = false;
                        pending_command = false;
                        command_ai_pending = false;
                        command_utterances.clear();
                        command_fragment = None;
                        command_queue.clear();
                        command_feedback_until = None;
                        session_id = session_id.wrapping_add(1);
                        let language = app
                            .state::<WebState>()
                            .config
                            .lock()
                            .ok()
                            .map(|config| config.source_language.clone())
                            .unwrap_or_else(|| "cn".into());
                        show_voice(
                            &app,
                            VoiceDisplay {
                                result: command_exit_feedback(&language).into(),
                                ..Default::default()
                            },
                        );
                        hide_at = Some(Instant::now() + Duration::from_secs(2));
                    } else {
                        if let Some(active) = &mut session {
                            active.stop();
                            suppress_voice_events = true;
                        }
                        pending_command = true;
                        command_mode = false;
                        command_ai_pending = false;
                        command_utterances.clear();
                        command_fragment = None;
                        command_queue.clear();
                        command_feedback_until = None;
                    }
                } else if matches!(control, VoiceControl::Release) {
                    if hold_started.is_some()
                        && !command_mode
                        && let Some(active) = &mut session
                    {
                        active.stop();
                    }
                } else if command_mode || pending_command {
                    if let Some(active) = &mut session {
                        active.cancel_for_handoff();
                        suppress_voice_events = true;
                    }
                    command_mode = false;
                    pending_command = false;
                    command_ai_pending = false;
                    command_utterances.clear();
                    command_fragment = None;
                    command_queue.clear();
                    command_feedback_until = None;
                    pending_normal_voice = true;
                    resume_command_after_voice = true;
                    session_id = session_id.wrapping_add(1);
                    show_voice(
                        &app,
                        VoiceDisplay {
                            status: "正在切换到普通语音输入…".into(),
                            ..Default::default()
                        },
                    );
                } else if let Some(active) = &mut session {
                    pending_command = false;
                    command_mode = false;
                    command_ai_pending = false;
                    active.stop();
                    show_voice(
                        &app,
                        VoiceDisplay {
                            original: String::new(),
                            result: String::new(),
                            status: "正在停止语音识别…".into(),
                        },
                    );
                    hide_at = Some(Instant::now() + Duration::from_secs(1));
                } else {
                    command_mode = false;
                    pending_command = false;
                    pending_normal_voice = false;
                    resume_command_after_voice = false;
                    command_ai_pending = false;
                    command_utterances.clear();
                    command_fragment = None;
                    command_queue.clear();
                    command_feedback_until = None;
                    session_id = session_id.wrapping_add(1);
                    latest_utterance = 0;
                    utterance_starts.clear();
                    ai_pending = 0;
                    context.clear();
                    match begin_normal_recording(&app, false) {
                        Ok((started, hold)) => {
                            session = Some(started);
                            suppress_voice_events = false;
                            hold_started = hold;
                            hide_at = None;
                        }
                        Err(reason) => {
                            eprintln!("无法开始语音识别：{reason:#}");
                            show_voice(
                                &app,
                                VoiceDisplay {
                                    original: String::new(),
                                    result: String::new(),
                                    status: format!("无法开始语音识别：{reason:#}"),
                                },
                            );
                            hide_at = Some(Instant::now() + Duration::from_secs(4));
                        }
                    }
                }
            }
            if let Some(started) = hold_started {
                if session.is_some() && started.elapsed() >= Duration::from_millis(80) {
                    let binding = app
                        .state::<WebState>()
                        .config
                        .lock()
                        .ok()
                        .map(|config| config.hotkeys.voice.clone())
                        .unwrap_or_default();
                    if !voice_hotkey_is_down(&binding) {
                        if let Some(active) = &mut session {
                            active.stop();
                        }
                        hold_started = None;
                    }
                }
            }
            let events = session
                .as_ref()
                .map(|active| active.receiver.try_iter().collect::<Vec<_>>())
                .unwrap_or_default();
            for event in events {
                if suppress_voice_events
                    && !matches!(event, VoiceEvent::Stopped(_) | VoiceEvent::CaptureReleased)
                {
                    continue;
                }
                match event {
                    VoiceEvent::CaptureReleased => {
                        session = None;
                        hold_started = None;
                        suppress_voice_events = false;
                        // The remaining batch belongs to the abandoned session.
                        break;
                    }
                    VoiceEvent::SpeechStarted(id) => {
                        latest_utterance = id;
                        utterance_starts.insert(id, Instant::now());
                        if command_mode {
                            if let Some((_, since)) = &mut command_fragment {
                                *since = Instant::now();
                            }
                            continue;
                        }
                        show_voice(
                            &app,
                            VoiceDisplay {
                                original: String::new(),
                                result: String::new(),
                                status: if command_mode {
                                    "请说出要修改的设置…"
                                } else {
                                    "正在聆听…"
                                }
                                .into(),
                            },
                        );
                        hide_at = None;
                    }
                    VoiceEvent::Recognizing(id) if id == latest_utterance => {
                        if command_mode {
                            show_voice(
                                &app,
                                VoiceDisplay {
                                    status: "正在识别指令…".into(),
                                    ..Default::default()
                                },
                            );
                            continue;
                        }
                        show_voice(
                            &app,
                            VoiceDisplay {
                                original: String::new(),
                                result: String::new(),
                                status: "正在识别…".into(),
                            },
                        );
                    }
                    VoiceEvent::Utterance(id, Ok(text)) => {
                        let original = text.trim().to_owned();
                        let started = utterance_starts.remove(&id).unwrap_or_else(Instant::now);
                        if command_mode {
                            let language = app
                                .state::<WebState>()
                                .config
                                .lock()
                                .ok()
                                .map(|config| config.source_language.clone())
                                .unwrap_or_else(|| "cn".into());
                            if settings_command::is_exit_phrase(&original, &language) {
                                if let Some(active) = &mut session {
                                    active.stop();
                                    suppress_voice_events = true;
                                }
                                command_mode = false;
                                pending_command = false;
                                command_ai_pending = false;
                                command_utterances.clear();
                                command_fragment = None;
                                command_queue.clear();
                                command_feedback_until = None;
                                session_id = session_id.wrapping_add(1);
                                show_voice(
                                    &app,
                                    VoiceDisplay {
                                        original,
                                        result: command_exit_feedback(&language).into(),
                                        ..Default::default()
                                    },
                                );
                                hide_at = Some(Instant::now() + Duration::from_secs(2));
                                continue;
                            }
                            let original = if let Some((fragment, since)) = command_fragment.take()
                            {
                                if since.elapsed() < Duration::from_secs(5) {
                                    format!("{fragment} {original}")
                                } else {
                                    original
                                }
                            } else {
                                original
                            };
                            if settings_command::is_start_voice_phrase(&original) {
                                command_feedback_until = None;
                                command_queue.push_back(QueuedSettingCommand {
                                    recognized: original,
                                    change: settings_command::SettingChange {
                                        setting: "action.start_voice".into(),
                                        value: serde_json::Value::Null,
                                        feedback: command_voice_start_feedback(&language).into(),
                                    },
                                });
                                continue;
                            }
                            if settings_command::incomplete_shortcut_request(&original) {
                                command_fragment = Some((original.clone(), Instant::now()));
                                show_voice(
                                    &app,
                                    VoiceDisplay {
                                        original,
                                        status: "请继续说出快捷键组合…".into(),
                                        ..Default::default()
                                    },
                                );
                                continue;
                            }
                            if voice_word_cache::is_obviously_bad(&original) {
                                if command_feedback_until.is_none() {
                                    show_voice(
                                        &app,
                                        VoiceDisplay {
                                            status: "没有识别到清晰的指令，请继续说话".into(),
                                            ..Default::default()
                                        },
                                    );
                                    command_feedback_until =
                                        Some(Instant::now() + Duration::from_secs(2));
                                }
                                continue;
                            }
                            if command_utterances.len() < 32 {
                                command_feedback_until = None;
                                show_voice(
                                    &app,
                                    VoiceDisplay {
                                        original: original.clone(),
                                        status: "正在理解指令…".into(),
                                        ..Default::default()
                                    },
                                );
                                command_utterances.push_back(original);
                            } else if command_feedback_until.is_none() {
                                show_voice(
                                    &app,
                                    VoiceDisplay {
                                        status: "指令队列已满，请稍后继续".into(),
                                        ..Default::default()
                                    },
                                );
                                command_feedback_until =
                                    Some(Instant::now() + Duration::from_secs(2));
                            }
                            continue;
                        }
                        if is_recent_tts_echo(app.state::<WebState>().inner(), &original, started) {
                            if let Some(active) = &session {
                                active.trace.record(id, "self_tts_echo", None, "filtered");
                            }
                            if id == latest_utterance {
                                show_voice(
                                    &app,
                                    VoiceDisplay {
                                        original: String::new(),
                                        result: String::new(),
                                        status: "已忽略扬声器回放".into(),
                                    },
                                );
                                hide_at = Some(Instant::now() + Duration::from_secs(1));
                            }
                            continue;
                        }
                        if id == latest_utterance {
                            show_voice(
                                &app,
                                VoiceDisplay {
                                    original: original.clone(),
                                    result: String::new(),
                                    status: "正在查询本地词库…".into(),
                                },
                            );
                        }
                        let previous = context.back().cloned();
                        context.push_back(original.clone());
                        if context.len() > 20 {
                            context.pop_front();
                        }
                        let config = match app.state::<WebState>().config.lock() {
                            Ok(config) => config.clone(),
                            Err(_) => continue,
                        };
                        if process_voice_text(
                            &app,
                            &mut lookup,
                            &config,
                            session_id,
                            id,
                            original,
                            previous,
                            &translation_tx,
                        ) {
                            ai_pending += 1;
                            hide_at = None;
                        } else {
                            hide_at = Some(Instant::now() + Duration::from_secs(1));
                        }
                    }
                    VoiceEvent::Utterance(id, Err(reason)) => {
                        utterance_starts.remove(&id);
                        if command_mode {
                            if command_feedback_until.is_none() {
                                show_voice(
                                    &app,
                                    VoiceDisplay {
                                        status: format!("指令识别失败：{reason}"),
                                        ..Default::default()
                                    },
                                );
                                command_feedback_until =
                                    Some(Instant::now() + Duration::from_secs(2));
                            }
                            continue;
                        }
                        show_voice(
                            &app,
                            VoiceDisplay {
                                original: String::new(),
                                result: String::new(),
                                status: format!("语音识别失败：{reason}"),
                            },
                        );
                        hide_at = Some(Instant::now() + Duration::from_secs(1));
                    }
                    VoiceEvent::Filtered(id) => {
                        utterance_starts.remove(&id);
                        if command_mode {
                            if command_feedback_until.is_none() {
                                show_voice(
                                    &app,
                                    VoiceDisplay {
                                        status: "不是已注册的本人声音，已跳过".into(),
                                        ..Default::default()
                                    },
                                );
                                command_feedback_until =
                                    Some(Instant::now() + Duration::from_secs(2));
                            }
                            continue;
                        }
                        if id == latest_utterance {
                            show_voice(
                                &app,
                                VoiceDisplay {
                                    original: String::new(),
                                    result: String::new(),
                                    status: "不是已注册的本人声音，已跳过".into(),
                                },
                            );
                            hide_at = Some(Instant::now() + Duration::from_secs(1));
                        }
                    }
                    VoiceEvent::Stopped(reason) => {
                        session = None;
                        hold_started = None;
                        if suppress_voice_events {
                            suppress_voice_events = false;
                            continue;
                        }
                        if resume_command_after_voice {
                            continue;
                        }
                        if command_mode {
                            command_mode = false;
                            command_ai_pending = false;
                            command_utterances.clear();
                            command_fragment = None;
                            command_queue.clear();
                            command_feedback_until = None;
                            session_id = session_id.wrapping_add(1);
                            show_voice(
                                &app,
                                VoiceDisplay {
                                    status: reason.map_or_else(
                                        || "指令模式录音已停止".into(),
                                        |reason| format!("指令模式录音中断：{reason}"),
                                    ),
                                    ..Default::default()
                                },
                            );
                            hide_at = Some(Instant::now() + Duration::from_secs(3));
                            continue;
                        }
                        if let Some(reason) = reason {
                            show_voice(
                                &app,
                                VoiceDisplay {
                                    original: String::new(),
                                    result: String::new(),
                                    status: format!("录音失败：{reason}"),
                                },
                            );
                            hide_at = Some(Instant::now() + Duration::from_secs(1));
                        } else if ai_pending == 0 && !command_ai_pending && hide_at.is_none() {
                            hide_at = Some(Instant::now() + Duration::from_secs(1));
                        }
                    }
                    _ => {}
                }
            }
            if command_mode
                && command_fragment
                    .as_ref()
                    .is_some_and(|(_, since)| since.elapsed() >= Duration::from_secs(5))
                && !utterance_starts.contains_key(&latest_utterance)
                && let Some((fragment, _)) = command_fragment.take()
            {
                if command_utterances.len() < 32 {
                    command_utterances.push_back(fragment);
                }
            }
            if pending_normal_voice && session.is_none() {
                pending_normal_voice = false;
                session_id = session_id.wrapping_add(1);
                latest_utterance = 0;
                utterance_starts.clear();
                ai_pending = 0;
                context.clear();
                match begin_normal_recording(&app, true) {
                    Ok((started, hold)) => {
                        session = Some(started);
                        suppress_voice_events = false;
                        hold_started = hold;
                        hide_at = None;
                    }
                    Err(reason) => {
                        show_voice(
                            &app,
                            VoiceDisplay {
                                status: format!("无法开始语音识别：{reason}"),
                                ..Default::default()
                            },
                        );
                        hide_at = Some(Instant::now() + Duration::from_secs(4));
                        if resume_command_after_voice {
                            resume_command_after_voice = false;
                            pending_command = true;
                        }
                    }
                }
            }
            if pending_command && session.is_none() {
                pending_command = false;
                session_id = session_id.wrapping_add(1);
                latest_utterance = 0;
                utterance_starts.clear();
                command_fragment = None;
                ai_pending = 0;
                context.clear();
                match begin_command_recording(&app) {
                    Ok(started) => {
                        session = Some(started);
                        suppress_voice_events = false;
                        command_mode = true;
                        hold_started = None;
                        hide_at = None;
                        show_voice(
                            &app,
                            VoiceDisplay {
                                status: "请说出要修改的设置…".into(),
                                ..Default::default()
                            },
                        );
                    }
                    Err(reason) => {
                        show_voice(
                            &app,
                            VoiceDisplay {
                                status: format!("指令模式无法启动：{reason}"),
                                ..Default::default()
                            },
                        );
                        hide_at = Some(Instant::now() + Duration::from_secs(4));
                    }
                }
            }
            for interpreted in command_rx.try_iter() {
                if interpreted.session_id != session_id || !command_mode {
                    continue;
                }
                command_ai_pending = false;
                match interpreted.result {
                    Ok(changes) if changes.is_empty() => {
                        show_voice(
                            &app,
                            VoiceDisplay {
                                original: interpreted.recognized,
                                status: "没有明确的设置指令，请继续说话".into(),
                                ..Default::default()
                            },
                        );
                        command_feedback_until = Some(Instant::now() + Duration::from_secs(2));
                    }
                    Ok(changes) => {
                        for change in changes {
                            command_queue.push_back(QueuedSettingCommand {
                                recognized: interpreted.recognized.clone(),
                                change,
                            });
                        }
                    }
                    Err(reason) => {
                        show_voice(
                            &app,
                            VoiceDisplay {
                                original: interpreted.recognized,
                                status: format!("指令未执行：{reason}"),
                                ..Default::default()
                            },
                        );
                        command_feedback_until = Some(Instant::now() + Duration::from_secs(3));
                    }
                }
            }
            if command_mode {
                if command_feedback_until.is_some_and(|deadline| Instant::now() >= deadline) {
                    command_feedback_until = None;
                    if command_queue.is_empty()
                        && command_utterances.is_empty()
                        && !command_ai_pending
                    {
                        show_voice(
                            &app,
                            VoiceDisplay {
                                status: "指令模式持续聆听中…".into(),
                                ..Default::default()
                            },
                        );
                    }
                }
                if command_feedback_until.is_none() {
                    if let Some(queued) = command_queue.pop_front() {
                        if queued.change.setting == "command_mode" {
                            let language = app
                                .state::<WebState>()
                                .config
                                .lock()
                                .ok()
                                .map(|config| config.source_language.clone())
                                .unwrap_or_else(|| "cn".into());
                            let feedback = command_success_feedback(
                                &queued.change,
                                &language,
                                "指令模式已关闭",
                            );
                            if let Some(active) = &mut session {
                                active.stop();
                                suppress_voice_events = true;
                            }
                            command_mode = false;
                            command_ai_pending = false;
                            command_utterances.clear();
                            command_fragment = None;
                            command_queue.clear();
                            session_id = session_id.wrapping_add(1);
                            show_voice(
                                &app,
                                VoiceDisplay {
                                    original: queued.recognized,
                                    result: feedback,
                                    ..Default::default()
                                },
                            );
                            hide_at = Some(Instant::now() + Duration::from_secs(2));
                        } else if queued.change.setting.starts_with("action.") {
                            let outcome = settings_command::parse_action(&queued.change)
                                .map_err(error)
                                .and_then(|action| action.ok_or("无法识别操作指令".into()));
                            match outcome {
                                Ok(settings_command::CommandAction::StartVoice)
                                    if !command_queue.is_empty() || command_ai_pending =>
                                {
                                    // Changing recorder modes ends command mode. Finish the
                                    // other commands from this utterance before that handoff.
                                    command_queue.push_back(queued);
                                }
                                Ok(settings_command::CommandAction::StartVoice) => {
                                    let language = app
                                        .state::<WebState>()
                                        .config
                                        .lock()
                                        .ok()
                                        .map(|config| config.source_language.clone())
                                        .unwrap_or_else(|| "cn".into());
                                    if let Some(active) = &mut session {
                                        active.cancel_for_handoff();
                                        suppress_voice_events = true;
                                    }
                                    command_mode = false;
                                    command_ai_pending = false;
                                    command_utterances.clear();
                                    command_fragment = None;
                                    command_queue.clear();
                                    command_feedback_until = None;
                                    pending_normal_voice = true;
                                    resume_command_after_voice = true;
                                    session_id = session_id.wrapping_add(1);
                                    show_voice(
                                        &app,
                                        VoiceDisplay {
                                            original: queued.recognized,
                                            result: command_success_feedback(
                                                &queued.change,
                                                &language,
                                                "正在打开普通语音识别",
                                            ),
                                            ..Default::default()
                                        },
                                    );
                                }
                                Ok(action) => {
                                    let language = app
                                        .state::<WebState>()
                                        .config
                                        .lock()
                                        .ok()
                                        .map(|config| config.source_language.clone())
                                        .unwrap_or_else(|| "cn".into());
                                    match execute_command_action(&app, action) {
                                        Ok(summary) => show_voice(
                                            &app,
                                            VoiceDisplay {
                                                original: queued.recognized,
                                                result: command_success_feedback(
                                                    &queued.change,
                                                    &language,
                                                    &summary,
                                                ),
                                                ..Default::default()
                                            },
                                        ),
                                        Err(reason) => show_voice(
                                            &app,
                                            VoiceDisplay {
                                                original: queued.recognized,
                                                status: format!("操作未完成：{reason}"),
                                                ..Default::default()
                                            },
                                        ),
                                    }
                                    command_feedback_until =
                                        Some(Instant::now() + Duration::from_secs(2));
                                }
                                Err(reason) => {
                                    show_voice(
                                        &app,
                                        VoiceDisplay {
                                            original: queued.recognized,
                                            status: format!("操作未执行：{reason}"),
                                            ..Default::default()
                                        },
                                    );
                                    command_feedback_until =
                                        Some(Instant::now() + Duration::from_secs(2));
                                }
                            }
                        } else {
                            let outcome = (|| -> Result<(String, String), String> {
                                let state = app.state::<WebState>();
                                let mut config = state.config.lock().map_err(error)?.clone();
                                let summary = settings_command::apply(
                                    &mut config,
                                    std::slice::from_ref(&queued.change),
                                )
                                .map_err(error)?;
                                if queued.change.setting == "voice_owner_filter"
                                    && config.voice_owner_filter
                                    && !voice_identity::is_registered()
                                {
                                    return Err("请先在设置中注册本人声纹".into());
                                }
                                let language = config.source_language.clone();
                                apply_config(config, &app, &state)?;
                                use tauri::Emitter;
                                let _ = app.emit_to(
                                    "settings",
                                    "voice-command-applied",
                                    vec![queued.change.setting.clone()],
                                );
                                Ok((summary, language))
                            })();
                            match outcome {
                                Ok((summary, language)) => show_voice(
                                    &app,
                                    VoiceDisplay {
                                        original: queued.recognized,
                                        result: command_success_feedback(
                                            &queued.change,
                                            &language,
                                            &summary,
                                        ),
                                        ..Default::default()
                                    },
                                ),
                                Err(reason) => show_voice(
                                    &app,
                                    VoiceDisplay {
                                        original: queued.recognized,
                                        status: format!("设置未更改：{reason}"),
                                        ..Default::default()
                                    },
                                ),
                            }
                            command_feedback_until = Some(Instant::now() + Duration::from_secs(2));
                        }
                    } else if !command_ai_pending
                        && let Some(original) = command_utterances.pop_front()
                    {
                        let language = app
                            .state::<WebState>()
                            .config
                            .lock()
                            .ok()
                            .map(|config| config.source_language.clone())
                            .unwrap_or_else(|| "cn".into());
                        let language = if language == "auto" {
                            "cn".to_string()
                        } else {
                            language
                        };
                        if settings_command::is_exit_phrase(&original, &language) {
                            command_queue.push_back(QueuedSettingCommand {
                                recognized: original,
                                change: settings_command::SettingChange {
                                    setting: "command_mode".into(),
                                    value: serde_json::Value::Bool(false),
                                    feedback: command_exit_feedback(&language).into(),
                                },
                            });
                        } else {
                            match credentials::load_key() {
                                Ok(Some(key)) if !key.trim().is_empty() => {
                                    show_voice(
                                        &app,
                                        VoiceDisplay {
                                            original: original.clone(),
                                            status: "正在理解设置指令…".into(),
                                            ..Default::default()
                                        },
                                    );
                                    let sender = command_tx.clone();
                                    let command_session_id = session_id;
                                    command_ai_pending = true;
                                    std::thread::spawn(move || {
                                        let result = ai::interpret_settings_command(
                                            &original, &key, &language,
                                        )
                                        .map_err(error);
                                        let _ = sender.send(CommandInterpretation {
                                            session_id: command_session_id,
                                            recognized: original,
                                            result,
                                        });
                                    });
                                }
                                _ => {
                                    show_voice(&app, VoiceDisplay {
                                        original,
                                        status: "指令模式需要 DeepSeek API Key，请继续说话或关闭模式".into(),
                                        ..Default::default()
                                    });
                                    command_feedback_until =
                                        Some(Instant::now() + Duration::from_secs(3));
                                }
                            }
                        }
                    }
                }
            }
            for translated in translation_rx.try_iter() {
                if translated.session_id != session_id || pending_command || command_mode {
                    continue;
                }
                ai_pending = ai_pending.saturating_sub(1);
                match translated.result {
                    Ok(candidate) => {
                        if let Some(sender) = app
                            .state::<WebState>()
                            .voice_cache
                            .lock()
                            .ok()
                            .and_then(|value| value.clone())
                        {
                            let _ = sender.send(CacheCommand::Candidate(VoiceCacheRequest {
                                candidate: candidate.clone(),
                                session_id,
                                previous: translated.previous,
                                next: None,
                            }));
                        }
                        let config = match app.state::<WebState>().config.lock() {
                            Ok(config) => config.clone(),
                            Err(_) => continue,
                        };
                        if translated.utterance_id == latest_utterance {
                            output_voice(
                                &app,
                                translated.recognized,
                                candidate.translated,
                                &config,
                            );
                            hide_at = Some(Instant::now() + Duration::from_secs(1));
                        }
                    }
                    Err(reason) if translated.utterance_id == latest_utterance => {
                        show_voice(
                            &app,
                            VoiceDisplay {
                                original: translated.recognized,
                                result: String::new(),
                                status: format!("AI 翻译失败：{reason}"),
                            },
                        );
                        hide_at = Some(Instant::now() + Duration::from_secs(1));
                    }
                    Err(_) => {}
                }
            }
            if resume_command_after_voice
                && session.is_none()
                && ai_pending == 0
                && hide_at.is_none_or(|deadline| Instant::now() >= deadline)
            {
                resume_command_after_voice = false;
                pending_command = true;
                hide_at = None;
            }
            if hide_at.is_some_and(|at| Instant::now() >= at) {
                hide_voice(&app);
                hide_at = None;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    });
}
