//! React/Tailwind 桌面界面的 Tauri 状态、窗口与快捷键调度。
//! 词库、OCR 和语音命令分别由同目录子模块实现。

mod browser_captions;
mod library;
mod ocr_runtime;
mod system_audio_runtime;
mod tts_runtime;
mod voice_runtime;

use self::browser_captions::*;
use self::library::*;
use self::ocr_runtime::*;
use self::system_audio_runtime::*;
use self::tts_runtime::*;
use self::voice_runtime::*;
use crate::domain::ai_word_save;
use crate::domain::shortcut::{MouseShortcut, Shortcut};
use crate::domain::storage::{self, StoredWord};
use crate::domain::voice_word_cache::{self, VoiceCacheRequest, VoiceWordCache};
use crate::domain::word_index::{MatchRank, SearchHit, WordIndex};
use crate::platform::credentials;
use crate::platform::extra_mouse_buttons::ExtraMouseButtons;
use crate::platform::mouse_hotkey::MouseHotkeyListener;
use crate::platform::narrator_voice;
use crate::platform::ocr::{self, CapturedScreen};
use crate::platform::piper_tts::{self, LocalTtsVoice};
use crate::platform::voice::{self, VoiceEvent, VoiceSession};
use crate::platform::voice_identity::{self, EnrollmentEvent};
use crate::platform::voice_output::VoiceInputTarget;
use crate::platform::window_monitor::{SettingsOrigin, settings_position};
use crate::platform::{tts_output, windows_tts};
use crate::web_config::{UiConfig, load_ui_config, save_ui_config};
use crate::{
    ai::{self, AiCandidate},
    project_directory, translation_language,
};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;
use std::time::Instant;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, State, WebviewUrl, WebviewWindowBuilder};

struct WebState {
    config: Mutex<UiConfig>,
    search: Mutex<SearchState>,
    hotkey_version: AtomicU64,
    ocr_screen: Mutex<Option<Arc<CapturedScreen>>>,
    ocr_original: Mutex<Option<String>>,
    ocr_generation: AtomicU64,
    ocr_request: AtomicU64,
    voice_control: Mutex<Option<Sender<VoiceControl>>>,
    tts_control: Mutex<Option<Sender<TtsCommand>>>,
    tts_generation: Arc<AtomicU64>,
    recent_tts: Mutex<VecDeque<RecentTts>>,
    voice_cache: Mutex<Option<Sender<CacheCommand>>>,
    voice_display: Mutex<VoiceDisplay>,
    system_audio_display: Mutex<VoiceDisplay>,
    browser_caption: Arc<Mutex<BrowserCaptionState>>,
    pending_query: Mutex<Option<String>>,
    shortcut_recording: Mutex<Option<Instant>>,
}

#[derive(Default)]
struct SearchState {
    library: String,
    index: WordIndex,
    previous_query: String,
    previous_hits: Vec<SearchHit>,
}

#[derive(Serialize)]
struct WebHit {
    key: String,
    value: String,
    pronunciation: String,
    library: String,
    rank: &'static str,
}

#[derive(Deserialize)]
struct UsedWord {
    library: String,
    key: String,
}

#[derive(Serialize)]
struct SearchPage {
    hits: Vec<WebHit>,
    total: usize,
    language_mismatch: bool,
}

#[derive(Serialize)]
struct LanguageOption {
    code: &'static str,
    name: &'static str,
}

#[derive(Serialize)]
struct WebTranslation {
    translated: String,
    pronunciation: String,
    save_error: Option<String>,
}

#[derive(Clone, Copy, Deserialize)]
struct WebRect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

#[derive(Clone, Serialize)]
struct OcrResult {
    original: String,
    translation: String,
    status: String,
    show_original: bool,
}

#[derive(Clone, Default, Serialize)]
struct VoiceDisplay {
    original: String,
    result: String,
    status: String,
}

enum VoiceControl {
    Toggle,
    Release,
}
enum CacheCommand {
    Candidate(VoiceCacheRequest),
    Review(Sender<Result<(), String>>),
}

struct VoiceTranslation {
    session_id: u64,
    utterance_id: u64,
    recognized: String,
    previous: Option<String>,
    result: Result<AiCandidate, String>,
}

#[derive(Default)]
struct VoiceLookup {
    library: String,
    index: WordIndex,
}

fn error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[tauri::command]
fn get_narrator_voice() -> Option<String> {
    narrator_voice::selected_voice()
}

#[tauri::command]
fn list_local_tts_voices() -> Vec<LocalTtsVoice> {
    piper_tts::list_voices()
}

#[tauri::command]
async fn list_windows_tts_voices(
    state: State<'_, WebState>,
) -> Result<Vec<windows_tts::WindowsTtsVoice>, String> {
    let sender = state
        .tts_control
        .lock()
        .map_err(error)?
        .clone()
        .ok_or("发音服务未启动")?;
    let (reply, receiver) = mpsc::channel();
    sender.send(TtsCommand::ListVoices(reply)).map_err(error)?;
    tauri::async_runtime::spawn_blocking(move || receiver.recv().map_err(error))
        .await
        .map_err(error)??
}

#[tauri::command]
fn list_audio_output_devices() -> Result<Vec<tts_output::AudioOutputDevice>, String> {
    tts_output::list_devices().map_err(error)
}

#[tauri::command]
async fn synthesize_windows(
    voice_id: String,
    text: String,
    state: State<'_, WebState>,
) -> Result<String, String> {
    use base64::Engine as _;
    let sender = state
        .tts_control
        .lock()
        .map_err(error)?
        .clone()
        .ok_or("发音服务未启动")?;
    let (reply, receiver) = mpsc::channel();
    sender
        .send(TtsCommand::Synthesize {
            voice_id,
            text,
            reply,
        })
        .map_err(error)?;
    let (samples, rate) =
        tauri::async_runtime::spawn_blocking(move || receiver.recv().map_err(error))
            .await
            .map_err(error)???;
    tauri::async_runtime::spawn_blocking(move || {
        let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
        let pcm_len = (samples.len() * 2) as u32;
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36_u32 + pcm_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt \x10\0\0\0\x01\0\x01\0");
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&(rate * 2).to_le_bytes());
        bytes.extend_from_slice(&[2, 0, 16, 0]);
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&pcm_len.to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(
                &((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).to_le_bytes(),
            );
        }
        Ok(format!(
            "data:audio/wav;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ))
    })
    .await
    .map_err(error)?
}

#[tauri::command]
async fn synthesize_piper(voice_id: String, text: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || piper_tts::synthesize(&voice_id, &text))
        .await
        .map_err(error)?
        .map_err(error)
}

fn library_name(source: &str, target: &str) -> String {
    translation_language::word_file_name(source, target)
}

#[tauri::command]
fn get_config(state: State<'_, WebState>) -> Result<UiConfig, String> {
    state
        .config
        .lock()
        .map(|value| value.clone())
        .map_err(error)
}

#[tauri::command]
fn get_default_config() -> UiConfig {
    UiConfig::default()
}

#[tauri::command]
fn save_config(config: UiConfig, app: AppHandle, state: State<'_, WebState>) -> Result<(), String> {
    use tauri::Emitter;
    let mut config = config.normalized();
    // The overlay saves its geometry independently while Settings may remain open.
    config.subtitle_bounds = state.config.lock().map_err(error)?.subtitle_bounds;
    validate_shortcuts(&config)?;
    if config.mic_tts_enabled && config.mic_output_device.is_empty() {
        return Err("请先选择虚拟音频播放设备，再开启自动麦克风发音".into());
    }
    save_ui_config(&config).map_err(error)?;
    let mic_tts_enabled = config.mic_tts_enabled;
    *state.config.lock().map_err(error)? = config;
    if !mic_tts_enabled {
        state.tts_generation.fetch_add(1, Ordering::Relaxed);
    }
    if let Some(tray) = app.tray_by_id("to_words_tray") {
        let tooltip = if mic_tts_enabled {
            "to_words · 自动麦克风发音已开启"
        } else {
            "to_words · 自动麦克风发音已关闭"
        };
        let _ = tray.set_tooltip(Some(tooltip));
    }
    state.hotkey_version.fetch_add(1, Ordering::Relaxed);
    let _ = app.emit_to("query", "config-updated", ());
    let _ = app.emit_to("settings", "config-updated", ());
    let _ = app.emit_to("system_audio", "config-updated", ());
    Ok(())
}

#[tauri::command]
fn set_shortcut_recording(
    active: bool,
    app: AppHandle,
    state: State<'_, WebState>,
) -> Result<(), String> {
    *state.shortcut_recording.lock().map_err(error)? = active.then(Instant::now);
    state.hotkey_version.fetch_add(1, Ordering::Relaxed);
    app.run_on_main_thread(|| {}).map_err(error)?;
    Ok(())
}

#[tauri::command]
fn voice_engine_status(
    backend: String,
    source: String,
    target: String,
    state: State<'_, WebState>,
) -> String {
    if let Ok(config) = state.config.lock() {
        crate::i18n::set_language(&config.ui_language);
    }
    let active = voice::effective_voice_backend(&backend, &source, &target);
    let status = voice::engine_status(active);
    if active == "sensevoice"
        && translation_language::same_language_pair(&source, &target).is_some()
    {
        format!(
            "{status}\n{}",
            crate::i18n::tr(
                "SenseVoice 自动检测语种；所有语言组合都会沿用已选引擎。同语言模式会过滤明显不符的文字，但不能保证固定识别该语言。"
            )
        )
    } else {
        status
    }
}

#[tauri::command]
fn has_api_key() -> Result<bool, String> {
    credentials::load_key()
        .map(|key| key.is_some())
        .map_err(error)
}

#[tauri::command]
fn set_api_key(key: String) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("请输入 Base64 编码的 API Key".to_string());
    }
    credentials::save_key(key).map_err(error)
}

#[tauri::command]
fn delete_api_key() -> Result<(), String> {
    credentials::delete_key().map_err(error)
}

fn validate_shortcuts(config: &UiConfig) -> Result<(), String> {
    let shortcuts = [
        &config.hotkeys.popup,
        &config.hotkeys.settings,
        &config.hotkeys.clear_composed,
        &config.hotkeys.ocr,
        &config.hotkeys.voice,
        &config.hotkeys.mic_tts,
        &config.hotkeys.system_audio,
    ];
    let mut unique = HashSet::new();
    for shortcut in shortcuts {
        let parsed = shortcut.parse::<Shortcut>()?;
        if !unique.insert(shortcut.to_ascii_lowercase()) {
            return Err("快捷键不能使用相同的组合".to_string());
        }
        let _ = parsed;
    }
    Ok(())
}

#[tauri::command]
fn copy_text(
    text: String,
    used_entries: Vec<UsedWord>,
    state: State<'_, WebState>,
) -> Result<(), String> {
    write_clipboard(text.clone())?;
    if let Ok(config) = state.config.lock() {
        if let Err(reason) = queue_tts(&state, &config, &text) {
            eprintln!("自动麦克风发音未开始：{reason}");
        }
    }
    let uses: Vec<_> = used_entries
        .into_iter()
        .map(|entry| (entry.library, entry.key))
        .collect();
    match storage::record_word_uses(&uses) {
        Ok(recorded) => {
            if let Ok(mut search) = state.search.lock() {
                for (library, key) in recorded {
                    if search.library == library {
                        search.index.record_use(&key);
                    }
                }
            }
        }
        Err(reason) => eprintln!("无法记录词条使用次数：{reason:#}"),
    }
    Ok(())
}

fn write_clipboard(text: String) -> Result<(), String> {
    arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.set_text(text))
        .map_err(error)
}

#[tauri::command]
fn hide_query(app: AppHandle) -> Result<(), String> {
    app.get_webview_window("query")
        .ok_or("查询窗口不存在")?
        .hide()
        .map_err(error)
}

#[tauri::command]
fn resize_query(width: f64, height: f64, app: AppHandle) -> Result<(), String> {
    if !width.is_finite() || !height.is_finite() {
        return Err("窗口尺寸无效".to_string());
    }
    let window = app.get_webview_window("query").ok_or("查询窗口不存在")?;
    // The page reports physical pixels (CSS size * devicePixelRatio). Keeping
    // this physical avoids converting with the DPI of a different monitor.
    window
        .set_size(tauri::Size::Physical(tauri::PhysicalSize::new(
            width.clamp(80.0, 3400.0).ceil() as u32,
            height.clamp(18.0, 2600.0).ceil() as u32,
        )))
        .map_err(error)?;
    sync_query_webview_size(&window)?;
    if let (Ok(Some(monitor)), Ok(position), Ok(size)) = (
        window.current_monitor(),
        window.outer_position(),
        window.outer_size(),
    ) {
        let bounds = monitor.position();
        let area = monitor.size();
        let max_x = bounds.x + area.width as i32 - size.width as i32;
        let max_y = bounds.y + area.height as i32 - size.height as i32 - 8;
        let x = position.x.clamp(bounds.x, max_x.max(bounds.x));
        let y = position.y.clamp(bounds.y, max_y.max(bounds.y));
        if (x, y) != (position.x, position.y) {
            window
                .set_position(tauri::Position::Physical(tauri::PhysicalPosition::new(
                    x, y,
                )))
                .map_err(error)?;
        }
    }
    Ok(())
}

fn set_query_size(window: &tauri::WebviewWindow, width: f64, height: f64) -> Result<(), String> {
    let size = tauri::Size::Logical(tauri::LogicalSize::new(width, height));
    window.set_size(size).map_err(error)?;
    sync_query_webview_size(window)
}

fn sync_query_webview_size(window: &tauri::WebviewWindow) -> Result<(), String> {
    // WebView2 is a separate child surface. On Windows, resizing its parent
    // (especially while the popup is hidden) does not always resize that
    // surface immediately. Use the *actual* client size, not the requested
    // logical size, so fractional display scaling cannot clip two edges.
    let size = tauri::Size::Physical(window.inner_size().map_err(error)?);
    let webview: &tauri::Webview = window.as_ref();
    webview.set_size(size).map_err(error)
}

#[tauri::command]
fn drag_query(app: AppHandle) -> Result<(), String> {
    app.get_webview_window("query")
        .ok_or("查询窗口不存在")?
        .start_dragging()
        .map_err(error)
}

#[tauri::command]
fn take_pending_query(state: State<'_, WebState>) -> Result<Option<String>, String> {
    Ok(state.pending_query.lock().map_err(error)?.take())
}

#[tauri::command]
fn exit_app(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
async fn translate_text(
    text: String,
    state: State<'_, WebState>,
) -> Result<WebTranslation, String> {
    let config = state.config.lock().map_err(error)?.clone();
    let text = text.trim().to_owned();
    if text.is_empty() {
        return Err("请输入要翻译的文字".to_string());
    }
    let source = if config.source_language == "auto" {
        translation_language::detect_source_language(&text)
            .unwrap_or("cn")
            .to_owned()
    } else {
        config.source_language.clone()
    };
    let target = config.translation_language;
    if source == target {
        if !translation_language::accepts_same_language_text(&text, &source) {
            return Err(format!(
                "输入内容不是所选的{}",
                translation_language::language_label(&source)
            ));
        }
        return Ok(WebTranslation {
            translated: text,
            pronunciation: String::new(),
            save_error: None,
        });
    }
    let key = credentials::load_key()
        .map_err(error)?
        .ok_or("请先在设置中填写 DeepSeek API Key")?;
    let politeness = config.ai_politeness;
    let auto_save = config.ai_auto_save;
    let result = tauri::async_runtime::spawn_blocking(move || {
        let candidate =
            ai::translate(text, source.clone(), target, source, key, politeness).map_err(error)?;
        let save_error = if auto_save {
            ai_word_save::save_candidate(&candidate).err().map(error)
        } else {
            None
        };
        Ok::<_, String>(WebTranslation {
            translated: candidate.translated,
            pronunciation: candidate.pronunciation,
            save_error,
        })
    })
    .await
    .map_err(error)??;
    state.search.lock().map_err(error)?.library.clear();
    Ok(result)
}

fn open_settings(app: &AppHandle, origin: SettingsOrigin) {
    if let Some(window) = app.get_webview_window("settings") {
        // 只在首次显示时按唤起位置居中；重复唤起保留用户调整过的位置。
        if !window.is_visible().unwrap_or(false) {
            if let Ok(size) = window.outer_size() {
                if let Some((x, y)) = settings_position(origin, (size.width, size.height)) {
                    let _ = window.set_position(tauri::Position::Physical(
                        tauri::PhysicalPosition::new(x, y),
                    ));
                }
            }
        }
        if window.is_minimized().unwrap_or(false) {
            let _ = window.unminimize();
        }
        let _ = window.show();
        let _ = window.set_focus();
        #[cfg(windows)]
        raise_settings_window(&window);
    }
}

#[cfg(windows)]
fn raise_settings_window(window: &tauri::WebviewWindow) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, HWND_NOTOPMOST, HWND_TOPMOST, SWP_NOMOVE, SWP_NOSIZE,
        SetForegroundWindow, SetWindowPos,
    };
    let Ok(handle) = window.hwnd() else { return };
    let hwnd = handle.0 as windows_sys::Win32::Foundation::HWND;
    unsafe {
        if GetForegroundWindow() == hwnd {
            return;
        }
        // 用户刚通过全局快捷键请求了这个窗口；先尝试正常激活。
        let _ = SetForegroundWindow(hwnd);
        if GetForegroundWindow() != hwnd {
            // Windows 有时拒绝后台窗口激活。短暂提升层级后立即还原，
            // 不让设置页永久保持“始终置顶”。
            let flags = SWP_NOMOVE | SWP_NOSIZE;
            let _ = SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, flags);
            let _ = SetWindowPos(hwnd, HWND_NOTOPMOST, 0, 0, 0, 0, flags);
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

fn open_query(app: &AppHandle) {
    use tauri::Emitter;
    if let Some(window) = app.get_webview_window("query") {
        let config = app
            .state::<WebState>()
            .config
            .lock()
            .ok()
            .map(|config| config.clone());
        if let Some(config) = config {
            // Match the 2px inset on every side of the query content. Drawing
            // the border on the WebView edge clips it on some Windows setups.
            let width = config.input.width + 4.0;
            let height = config.input.height + 4.0;
            let _ = set_query_size(&window, width as f64, height as f64);
            if let Some((x, y)) = crate::web_config::web_query_position(
                width,
                height,
                window.scale_factor().unwrap_or(1.0) as f32,
            ) {
                let _ = window.set_position(tauri::Position::Physical(
                    tauri::PhysicalPosition::new(x, y),
                ));
            }
        }
        let _ = window.show();
        let _ = sync_query_webview_size(&window);
        let _ = window.set_focus();
        let _ = app.emit_to("query", "query-opened", ());
    }
}

fn create_tray(app: &tauri::App) -> anyhow::Result<()> {
    let query = MenuItem::with_id(app, "query", "打开查询", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
    let exit = MenuItem::with_id(app, "exit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&query, &settings, &exit])?;
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("程序图标缺失"))?;
    let tts_enabled = app
        .state::<WebState>()
        .config
        .lock()
        .map_err(|reason| anyhow::anyhow!("无法读取语音设置：{reason}"))?
        .mic_tts_enabled;
    TrayIconBuilder::with_id("to_words_tray")
        .icon(icon)
        .tooltip(if tts_enabled {
            "to_words · 自动麦克风发音已开启"
        } else {
            "to_words · 自动麦克风发音已关闭"
        })
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "query" => open_query(app),
            "settings" => open_settings(app, SettingsOrigin::Mouse),
            "exit" => app.exit(0),
            _ => {}
        })
        .build(app)?;
    Ok(())
}

fn start_mouse_worker(app: AppHandle, window_handle: Option<isize>) {
    std::thread::spawn(move || {
        let Ok(listener) = MouseHotkeyListener::new() else {
            return;
        };
        let mut extra_buttons =
            window_handle.and_then(|handle| ExtraMouseButtons::new(handle).ok());
        loop {
            let mut events = listener.try_recv().into_iter().collect::<Vec<_>>();
            if let Some(extra) = &mut extra_buttons {
                events.extend(extra.poll());
            }
            if events.is_empty() {
                std::thread::sleep(Duration::from_millis(20));
            }
            for event in events {
                let recording = app
                    .state::<WebState>()
                    .shortcut_recording
                    .lock()
                    .ok()
                    .and_then(|since| *since);
                if recording.is_some() {
                    if event.pressed
                        && recording
                            .is_some_and(|since| since.elapsed() >= Duration::from_millis(180))
                    {
                        use tauri::Emitter;
                        let binding = MouseShortcut {
                            button: event.button,
                            modifiers: event.modifiers,
                        }
                        .to_string();
                        let _ = app.emit_to("settings", "mouse-shortcut", binding);
                    }
                    continue;
                }
                if !event.pressed {
                    let release_voice =
                        app.state::<WebState>()
                            .config
                            .lock()
                            .ok()
                            .is_some_and(|config| {
                                config.voice_hold_to_talk
                                    && config
                                        .hotkeys
                                        .voice
                                        .parse::<Shortcut>()
                                        .ok()
                                        .and_then(Shortcut::mouse)
                                        .is_some_and(|shortcut| shortcut.button == event.button)
                            });
                    if release_voice {
                        if let Ok(sender) = app.state::<WebState>().voice_control.lock() {
                            if let Some(sender) = sender.as_ref() {
                                let _ = sender.send(VoiceControl::Release);
                            }
                        }
                    }
                    continue;
                }
                let action = app
                    .state::<WebState>()
                    .config
                    .lock()
                    .ok()
                    .and_then(|config| {
                        [
                            (&config.hotkeys.popup, "query"),
                            (&config.hotkeys.settings, "settings"),
                            (&config.hotkeys.clear_composed, "clear"),
                            (&config.hotkeys.ocr, "ocr"),
                            (&config.hotkeys.voice, "voice"),
                            (&config.hotkeys.mic_tts, "mic_tts"),
                            (&config.hotkeys.system_audio, "system_audio"),
                        ]
                        .into_iter()
                        .find_map(|(value, action)| {
                            if let Ok(Shortcut::Mouse(shortcut)) = value.parse::<Shortcut>() {
                                shortcut
                                    .matches(event.button, event.modifiers)
                                    .then_some(action)
                            } else {
                                None
                            }
                        })
                    });
                match action {
                    Some("query") => open_query(&app),
                    Some("settings") => open_settings(&app, SettingsOrigin::Mouse),
                    Some("clear") => {
                        use tauri::Emitter;
                        let _ = app.emit_to("query", "clear-composed", ());
                    }
                    Some("ocr") => {
                        let _ = open_ocr(&app);
                    }
                    Some("voice") => {
                        let _ = toggle_voice(app.state::<WebState>());
                    }
                    Some("mic_tts") => toggle_mic_tts(&app),
                    Some("system_audio") => toggle_system_audio(&app),
                    _ => {}
                }
            }
        }
    });
}

fn register_keyboard_shortcuts(manager: &GlobalHotKeyManager, config: &UiConfig) {
    for (name, value) in [
        ("查询", &config.hotkeys.popup),
        ("设置", &config.hotkeys.settings),
        ("OCR", &config.hotkeys.ocr),
        ("语音识别", &config.hotkeys.voice),
        ("自动麦克风发音", &config.hotkeys.mic_tts),
        ("系统声音字幕与翻译", &config.hotkeys.system_audio),
    ] {
        match value.parse::<Shortcut>() {
            Ok(Shortcut::Keyboard(key)) => {
                if let Err(reason) = manager.register(key) {
                    eprintln!("{name}快捷键 {value} 注册失败：{reason}");
                }
            }
            Ok(Shortcut::Mouse(_)) => {}
            Err(reason) => eprintln!("{name}快捷键 {value} 无效：{reason}"),
        }
    }
}

fn keyboard_hotkey_action(config: &UiConfig, id: u32) -> Option<&'static str> {
    [
        (&config.hotkeys.popup, "query"),
        (&config.hotkeys.settings, "settings"),
        (&config.hotkeys.ocr, "ocr"),
        (&config.hotkeys.voice, "voice"),
        (&config.hotkeys.mic_tts, "mic_tts"),
        (&config.hotkeys.system_audio, "system_audio"),
    ]
    .into_iter()
    .find_map(|(binding, action)| match binding.parse::<Shortcut>() {
        Ok(Shortcut::Keyboard(key)) if key.id() == id => Some(action),
        _ => None,
    })
}

fn dispatch_keyboard_hotkey(app: &AppHandle, action: &str) {
    match action {
        "query" => open_query(app),
        "settings" => open_settings(app, SettingsOrigin::Keyboard),
        "ocr" => {
            let _ = open_ocr(app);
        }
        "voice" => {
            position_voice(app);
            let _ = toggle_voice(app.state::<WebState>());
        }
        "mic_tts" => toggle_mic_tts(app),
        "system_audio" => toggle_system_audio(app),
        _ => {}
    }
}

pub(crate) fn run() -> anyhow::Result<()> {
    let config = load_ui_config()?;
    let browser_caption = Arc::new(Mutex::new(BrowserCaptionState::default()));
    let app = tauri::Builder::default()
        .manage(WebState {
            config: Mutex::new(config),
            search: Mutex::new(SearchState::default()),
            hotkey_version: AtomicU64::new(0),
            ocr_screen: Mutex::new(None),
            ocr_original: Mutex::new(None),
            ocr_generation: AtomicU64::new(0),
            ocr_request: AtomicU64::new(0),
            voice_control: Mutex::new(None),
            tts_control: Mutex::new(None),
            tts_generation: Arc::new(AtomicU64::new(0)),
            recent_tts: Mutex::new(VecDeque::new()),
            voice_cache: Mutex::new(None),
            voice_display: Mutex::new(VoiceDisplay::default()),
            system_audio_display: Mutex::new(VoiceDisplay::default()),
            browser_caption: browser_caption.clone(),
            pending_query: Mutex::new(None),
            shortcut_recording: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            get_default_config,
            save_config,
            set_shortcut_recording,
            ocr_engine_status,
            voice_engine_status,
            list_languages,
            search_words,
            list_words,
            save_words,
            export_words,
            copy_text,
            hide_query,
            exit_app,
            translate_text,
            has_api_key,
            set_api_key,
            delete_api_key,
            resize_query,
            drag_query,
            get_ocr_snapshot,
            show_ocr,
            recognize_ocr,
            confirm_ocr,
            cancel_ocr,
            get_voice_display,
            get_system_audio_display,
            save_subtitle_bounds,
            get_narrator_voice,
            list_local_tts_voices,
            list_windows_tts_voices,
            list_audio_output_devices,
            synthesize_piper,
            synthesize_windows,
            set_mic_tts_enabled,
            take_pending_query,
            toggle_voice,
            review_voice_cache_now,
            voice_registered,
            delete_voice_profile,
            register_voice_profile,
        ])
        .setup(|app| {
            start_bridge(app.state::<WebState>().browser_caption.clone());
            WebviewWindowBuilder::new(
                app,
                "query",
                WebviewUrl::App("index.html?view=query".into()),
            )
            .title("to_words")
            .inner_size(620.0, 430.0)
            .decorations(false)
            .shadow(false)
            .transparent(true)
            .always_on_top(true)
            .visible(false)
            .build()?;
            // 预热 OCR WebView；第一次按快捷键时无需再创建浏览器窗口。
            create_ocr_window(app.handle())?;
            let voice_window = WebviewWindowBuilder::new(
                app,
                "voice",
                WebviewUrl::App("index.html?view=voice".into()),
            )
            .title("to_words 语音识别")
            .inner_size(900.0, 120.0)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .focusable(false)
            .visible(false)
            .build()?;
            let _ = voice_window.set_ignore_cursor_events(true);
            position_voice(app.handle());
            let system_audio_window = WebviewWindowBuilder::new(
                app,
                "system_audio",
                WebviewUrl::App("index.html?view=system_audio".into()),
            )
            .title("to_words 系统声音字幕")
            .inner_size(1020.0, 115.0)
            .min_inner_size(320.0, 70.0)
            .resizable(true)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .focusable(false)
            .visible(false)
            .build()?;
            if let Err(reason) = system_audio_window.set_ignore_cursor_events(false) {
                eprintln!("字幕窗口无法启用鼠标拖动：{reason}");
            }
            let (voice_tx, voice_rx) = mpsc::channel();
            let (tts_tx, tts_rx) = mpsc::channel();
            let (cache_tx, cache_rx) = mpsc::channel();
            *app.state::<WebState>()
                .voice_control
                .lock()
                .map_err(error)? = Some(voice_tx);
            *app.state::<WebState>().tts_control.lock().map_err(error)? = Some(tts_tx);
            *app.state::<WebState>().voice_cache.lock().map_err(error)? = Some(cache_tx);
            start_cache_worker(app.handle().clone(), cache_rx);
            start_voice_worker(app.handle().clone(), voice_rx);
            start_system_audio_worker(app.handle().clone());
            start_tts_worker(
                app.handle().clone(),
                tts_rx,
                app.state::<WebState>().tts_generation.clone(),
            );
            create_tray(app)?;
            #[cfg(windows)]
            let window_handle = app
                .get_webview_window("query")
                .and_then(|window| window.hwnd().ok().map(|handle| handle.0 as isize));
            #[cfg(not(windows))]
            let window_handle = None;
            start_mouse_worker(app.handle().clone(), window_handle);
            let hotkey_app = app.handle().clone();
            GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
                if event.state != HotKeyState::Pressed {
                    return;
                }
                let action = {
                    let state = hotkey_app.state::<WebState>();
                    if state
                        .shortcut_recording
                        .lock()
                        .ok()
                        .is_some_and(|value| value.is_some())
                    {
                        return;
                    }
                    state
                        .config
                        .lock()
                        .ok()
                        .and_then(|config| keyboard_hotkey_action(&config, event.id))
                };
                if let Some(action) = action {
                    let app = hotkey_app.clone();
                    let _ = hotkey_app
                        .run_on_main_thread(move || dispatch_keyboard_hotkey(&app, action));
                }
            }));
            Ok(())
        })
        .build(tauri::generate_context!())?;
    let mut version = u64::MAX;
    let mut manager: Option<GlobalHotKeyManager> = None;
    app.run(move |app, event| {
        if let tauri::RunEvent::MainEventsCleared = event {
            let state = app.state::<WebState>();
            let current = state.hotkey_version.load(Ordering::Relaxed);
            if current != version {
                version = current;
                manager = None;
                if state
                    .shortcut_recording
                    .lock()
                    .ok()
                    .is_some_and(|value| value.is_none())
                {
                    if let Ok(new_manager) = GlobalHotKeyManager::new() {
                        if let Ok(config) = state.config.lock() {
                            register_keyboard_shortcuts(&new_manager, &config);
                        }
                        manager = Some(new_manager);
                    }
                }
            }
        }
        if let tauri::RunEvent::WindowEvent {
            label,
            event: tauri::WindowEvent::Resized(_),
            ..
        } = &event
        {
            if label == "query" {
                if let Some(window) = app.get_webview_window(label) {
                    let _ = sync_query_webview_size(&window);
                }
            }
        }
        if let tauri::RunEvent::WindowEvent {
            label,
            event: tauri::WindowEvent::CloseRequested { api, .. },
            ..
        } = event
        {
            if label == "settings" || label == "query" || label == "ocr" {
                api.prevent_close();
                if label == "ocr" {
                    let _ = cancel_ocr(app.clone());
                } else if let Some(window) = app.get_webview_window(&label) {
                    let _ = window.hide();
                }
            }
        }
        let _ = &manager;
    });
    Ok(())
}
