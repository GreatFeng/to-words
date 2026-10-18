//! React/Tailwind 桌面界面的 Rust 命令与窗口运行时。
//! 保留 SQLite、词库索引、DeepSeek 凭据以及原有全局快捷键实现。

use crate::domain::ai_word_save;
use crate::domain::shortcut::{MouseShortcut, Shortcut};
use crate::domain::storage::{self, StoredWord};
use crate::domain::voice_word_cache::{self, VoiceCacheRequest, VoiceWordCache};
use crate::domain::word_index::{MatchRank, SearchHit, WordIndex};
use crate::platform::credentials;
use crate::platform::extra_mouse_buttons::ExtraMouseButtons;
use crate::platform::mouse_hotkey::MouseHotkeyListener;
use crate::platform::ocr::{self, CapturedScreen};
use crate::platform::voice::{self, VoiceEvent, VoiceSession};
use crate::platform::voice_identity::{self, EnrollmentEvent};
use crate::platform::voice_output::VoiceInputTarget;
use crate::web_config::{UiConfig, load_ui_config, save_ui_config};
use crate::{
    ai::{self, AiCandidate},
    project_directory, translation_language,
};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
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
    voice_cache: Mutex<Option<Sender<CacheCommand>>>,
    voice_display: Mutex<VoiceDisplay>,
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
    let config = config.normalized();
    validate_shortcuts(&config)?;
    save_ui_config(&config).map_err(error)?;
    *state.config.lock().map_err(error)? = config;
    state.hotkey_version.fetch_add(1, Ordering::Relaxed);
    let _ = app.emit_to("query", "config-updated", ());
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
fn ocr_engine_status(state: State<'_, WebState>) -> String {
    if let Ok(config) = state.config.lock() {
        crate::i18n::set_language(&config.ui_language);
    }
    ocr::engine_status()
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
        return Err("API Key 不能为空".to_string());
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
fn list_languages() -> Vec<LanguageOption> {
    translation_language::LANGUAGES
        .iter()
        .map(|item| LanguageOption {
            code: item.code,
            name: item.label,
        })
        .collect()
}

#[tauri::command]
fn search_words(
    query: String,
    page: usize,
    page_size: usize,
    state: State<'_, WebState>,
) -> Result<SearchPage, String> {
    let config = state.config.lock().map_err(error)?.clone();
    let query = query.trim();
    if query.is_empty() {
        return Ok(SearchPage {
            hits: Vec::new(),
            total: 0,
            language_mismatch: false,
        });
    }
    if let Some(language) = translation_language::same_language_pair(
        &config.source_language,
        &config.translation_language,
    ) {
        let valid = translation_language::accepts_same_language_text(query, language);
        return Ok(SearchPage {
            hits: if valid && page == 0 {
                vec![WebHit {
                    key: query.to_owned(),
                    value: String::new(),
                    library: String::new(),
                    rank: "exact",
                }]
            } else {
                Vec::new()
            },
            total: usize::from(valid),
            language_mismatch: !valid,
        });
    }
    let source = if config.source_language == "auto" {
        translation_language::detect_source_language(query).unwrap_or("cn")
    } else {
        &config.source_language
    };
    let library = library_name(source, &config.translation_language);
    let mut search = state.search.lock().map_err(error)?;
    if search.library != library {
        search.index = WordIndex::from_rows(storage::load_search_rows(&library).map_err(error)?);
        search.library = library.clone();
        search.previous_query.clear();
        search.previous_hits.clear();
    }
    let hits = search
        .index
        .search(query, &search.previous_query, &search.previous_hits);
    let total = hits.len();
    let start = page.saturating_mul(page_size.clamp(1, 100));
    let page_hits = hits
        .iter()
        .skip(start)
        .take(page_size.clamp(1, 100))
        .filter_map(|hit| {
            search.index.get(hit.entry_id).map(|entry| WebHit {
                key: entry.key.clone(),
                value: entry.value.clone(),
                library: library.clone(),
                rank: match hit.rank {
                    MatchRank::Exact => "exact",
                    MatchRank::Prefix => "prefix",
                    MatchRank::Contains => "contains",
                },
            })
        })
        .collect();
    search.previous_query = query.to_string();
    search.previous_hits = hits;
    Ok(SearchPage {
        hits: page_hits,
        total,
        language_mismatch: false,
    })
}

#[tauri::command]
fn list_words(source: String, target: String) -> Result<Vec<StoredWord>, String> {
    storage::list_words_newest(&library_name(&source, &target)).map_err(error)
}

#[tauri::command]
async fn review_voice_cache_now(state: State<'_, WebState>) -> Result<(), String> {
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

fn start_cache_worker(app: AppHandle, receiver: Receiver<CacheCommand>) {
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
fn save_words(
    source: String,
    target: String,
    rows: Vec<StoredWord>,
    state: State<'_, WebState>,
) -> Result<(), String> {
    let library = library_name(&source, &target);
    storage::save_words(&library, &rows).map_err(error)?;
    let mut search = state.search.lock().map_err(error)?;
    search.library.clear();
    search.previous_query.clear();
    search.previous_hits.clear();
    Ok(())
}

#[tauri::command]
fn export_words(name: String, rows: Vec<StoredWord>) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\', ':'])
        || name.chars().any(char::is_control)
    {
        return Err("文件名无效".to_string());
    }
    let name = if name.to_ascii_lowercase().ends_with(".json") {
        name.to_owned()
    } else {
        format!("{name}.json")
    };
    let path = project_directory().join(name);
    if path.exists() {
        return Err(format!("文件已存在，请使用其他名称：{}", path.display()));
    }
    let mut words = std::collections::BTreeMap::new();
    for row in rows {
        if !row.key.trim().is_empty() && !row.value.trim().is_empty() {
            words.insert(row.key, row.value);
        }
    }
    let content = serde_json::to_vec_pretty(&words).map_err(error)?;
    std::fs::write(&path, content).map_err(error)?;
    Ok(path.display().to_string())
}

#[tauri::command]
fn copy_text(
    text: String,
    used_entries: Vec<UsedWord>,
    state: State<'_, WebState>,
) -> Result<(), String> {
    write_clipboard(text)?;
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
fn get_ocr_snapshot(state: State<'_, WebState>) -> Result<String, String> {
    use base64::Engine;
    use image::{ColorType, ImageEncoder, codecs::png::PngEncoder};
    let screen = state
        .ocr_screen
        .lock()
        .map_err(error)?
        .clone()
        .ok_or("尚未截取屏幕")?;
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(
            &screen.rgb,
            screen.monitor.width,
            screen.monitor.height,
            ColorType::Rgb8.into(),
        )
        .map_err(error)?;
    Ok(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    ))
}

#[tauri::command]
async fn recognize_ocr(
    rect: WebRect,
    viewport_width: f32,
    viewport_height: f32,
    state: State<'_, WebState>,
) -> Result<OcrResult, String> {
    let generation = state.ocr_generation.load(Ordering::Relaxed);
    let request = state.ocr_request.fetch_add(1, Ordering::Relaxed) + 1;
    let screen = state
        .ocr_screen
        .lock()
        .map_err(error)?
        .clone()
        .ok_or("尚未截取屏幕")?;
    let config = state.config.lock().map_err(error)?.clone();
    let region = ocr::selected_region(
        screen.monitor,
        [viewport_width, viewport_height],
        [rect.x, rect.y],
        [rect.x + rect.width, rect.y + rect.height],
    )
    .ok_or("请框选至少 8×8 像素的区域")?;
    let source = config.source_language.clone();
    let target = config.translation_language.clone();
    let original = tauri::async_runtime::spawn_blocking(move || {
        ocr::recognize_selection(&screen, region, &source, &target).map_err(error)
    })
    .await
    .map_err(error)??;
    if state.ocr_generation.load(Ordering::Relaxed) != generation
        || state.ocr_request.load(Ordering::Relaxed) != request
    {
        return Err("OCR 选区已变化，请查看最新识别结果".to_string());
    }
    *state.ocr_original.lock().map_err(error)? = Some(original.clone());
    let translation_target = if config.source_language == "auto" {
        "cn".to_string()
    } else {
        config.source_language.clone()
    };
    if !config.ocr_auto_translate
        || config.source_language == config.translation_language
        || translation_language::detect_source_language(&original)
            == Some(translation_target.as_str())
    {
        return Ok(OcrResult {
            original,
            translation: String::new(),
            status: String::new(),
            show_original: config.ocr_show_original,
        });
    }
    let Some(key) = credentials::load_key().map_err(error)? else {
        return Ok(OcrResult {
            original,
            translation: String::new(),
            status: "未配置 DeepSeek API Key，已保留识别原文".into(),
            show_original: config.ocr_show_original,
        });
    };
    let text = original.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        ai::translate_ocr_text(&text, &translation_target, &key, config.ai_politeness)
            .map_err(error)
    })
    .await
    .map_err(error)?;
    match result {
        Ok(translation) => Ok(OcrResult {
            original,
            translation,
            status: String::new(),
            show_original: config.ocr_show_original,
        }),
        Err(message) => Ok(OcrResult {
            original,
            translation: String::new(),
            status: message,
            show_original: config.ocr_show_original,
        }),
    }
}

#[tauri::command]
fn confirm_ocr(app: AppHandle) -> Result<(), String> {
    let original = app
        .state::<WebState>()
        .ocr_original
        .lock()
        .map_err(error)?
        .clone()
        .ok_or("尚未识别到文字")?;
    write_clipboard(original)?;
    if let Some(window) = app.get_webview_window("ocr") {
        window.hide().map_err(error)?;
    }
    Ok(())
}

#[tauri::command]
fn cancel_ocr(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("ocr") {
        window.hide().map_err(error)?;
    }
    Ok(())
}

#[tauri::command]
fn get_voice_display(state: State<'_, WebState>) -> Result<VoiceDisplay, String> {
    state
        .voice_display
        .lock()
        .map(|value| value.clone())
        .map_err(error)
}

#[tauri::command]
fn take_pending_query(state: State<'_, WebState>) -> Result<Option<String>, String> {
    Ok(state.pending_query.lock().map_err(error)?.take())
}

#[tauri::command]
fn toggle_voice(state: State<'_, WebState>) -> Result<(), String> {
    state
        .voice_control
        .lock()
        .map_err(error)?
        .as_ref()
        .ok_or("语音服务尚未启动")?
        .send(VoiceControl::Toggle)
        .map_err(error)
}

#[cfg(windows)]
fn voice_hotkey_is_down(binding: &str) -> bool {
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
fn voice_hotkey_is_down(_binding: &str) -> bool {
    false
}

#[tauri::command]
fn voice_registered() -> bool {
    voice_identity::is_registered()
}

#[tauri::command]
fn delete_voice_profile(state: State<'_, WebState>) -> Result<(), String> {
    voice_identity::clear_profile().map_err(error)?;
    let mut config = state.config.lock().map_err(error)?;
    config.voice_owner_filter = false;
    save_ui_config(&config).map_err(error)
}

#[tauri::command]
async fn register_voice_profile(app: AppHandle, state: State<'_, WebState>) -> Result<(), String> {
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

fn open_ocr(app: &AppHandle) -> Result<(), String> {
    use tauri::Emitter;
    // 截屏先于覆盖层显示，避免把覆盖层自己拍进去。
    let monitor = ocr::monitor_under_cursor().map_err(error)?;
    let screen = Arc::new(ocr::capture_monitor(monitor).map_err(error)?);
    app.state::<WebState>()
        .ocr_generation
        .fetch_add(1, Ordering::Relaxed);
    *app.state::<WebState>().ocr_screen.lock().map_err(error)? = Some(screen);
    *app.state::<WebState>().ocr_original.lock().map_err(error)? = None;
    let window = if let Some(window) = app.get_webview_window("ocr") {
        window
    } else {
        WebviewWindowBuilder::new(app, "ocr", WebviewUrl::App("index.html?view=ocr".into()))
            .title("to_words OCR")
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .visible(false)
            .build()
            .map_err(error)?
    };
    window
        .set_position(tauri::Position::Physical(tauri::PhysicalPosition::new(
            monitor.x, monitor.y,
        )))
        .map_err(error)?;
    window
        .set_size(tauri::Size::Physical(tauri::PhysicalSize::new(
            monitor.width,
            monitor.height,
        )))
        .map_err(error)?;
    window.show().map_err(error)?;
    window.set_focus().map_err(error)?;
    let _ = app.emit_to("ocr", "ocr-reset", ());
    Ok(())
}

fn show_voice(app: &AppHandle, display: VoiceDisplay) {
    use tauri::Emitter;
    if let Ok(mut current) = app.state::<WebState>().voice_display.lock() {
        *current = display.clone();
    }
    if let Some(window) = app.get_webview_window("voice") {
        let _ = window.show();
        let _ = app.emit_to("voice", "voice-display", display);
    }
}

fn hide_voice(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("voice") {
        let _ = window.hide();
    }
}

fn position_voice(app: &AppHandle) {
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

fn type_voice_result(result: &str, config: &UiConfig) -> String {
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

fn output_voice(app: &AppHandle, original: String, result: String, config: &UiConfig) {
    let status = type_voice_result(&result, config);
    show_voice(
        app,
        VoiceDisplay {
            original,
            result,
            status,
        },
    );
}

fn process_voice_text(
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

fn start_voice_worker(app: AppHandle, controls: Receiver<VoiceControl>) {
    std::thread::spawn(move || {
        let (translation_tx, translation_rx) = mpsc::channel::<VoiceTranslation>();
        let mut session: Option<VoiceSession> = None;
        let mut session_id = 0_u64;
        let mut latest_utterance = 0_u64;
        let mut ai_pending = 0_usize;
        let mut lookup = VoiceLookup::default();
        let mut context = VecDeque::<String>::new();
        let mut hide_at: Option<Instant> = None;
        let mut hold_started: Option<Instant> = None;
        loop {
            if let Ok(control) = controls.try_recv() {
                if matches!(control, VoiceControl::Release) {
                    if let Some(active) = &mut session {
                        active.stop();
                    }
                } else if let Some(active) = &mut session {
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
                    let config = match app.state::<WebState>().config.lock() {
                        Ok(config) => config.clone(),
                        Err(_) => continue,
                    };
                    session_id = session_id.wrapping_add(1);
                    latest_utterance = 0;
                    ai_pending = 0;
                    context.clear();
                    position_voice(&app);
                    show_voice(
                        &app,
                        VoiceDisplay {
                            original: String::new(),
                            result: String::new(),
                            status: "正在启动语音识别…".into(),
                        },
                    );
                    match voice::start(
                        config.voice_keep_input || config.voice_hold_to_talk,
                        config.voice_hold_to_talk,
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
                    ) {
                        Ok(started) => {
                            started.trace.record(0, "voice_session_ready", None, "ok");
                            session = Some(started);
                            hold_started = config.voice_hold_to_talk.then(Instant::now);
                            position_voice(&app);
                            show_voice(
                                &app,
                                VoiceDisplay {
                                    original: String::new(),
                                    result: String::new(),
                                    status: "正在聆听…".into(),
                                },
                            );
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
                match event {
                    VoiceEvent::SpeechStarted(id) => {
                        latest_utterance = id;
                        show_voice(
                            &app,
                            VoiceDisplay {
                                original: String::new(),
                                result: String::new(),
                                status: "正在聆听…".into(),
                            },
                        );
                        hide_at = None;
                    }
                    VoiceEvent::Recognizing(id) if id == latest_utterance => {
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
                    VoiceEvent::Utterance(_, Err(reason)) => {
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
                    VoiceEvent::Filtered(id) if id == latest_utterance => {
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
                    VoiceEvent::Stopped(reason) => {
                        session = None;
                        hold_started = None;
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
                        } else if ai_pending == 0 && hide_at.is_none() {
                            hide_at = Some(Instant::now() + Duration::from_secs(1));
                        }
                    }
                    _ => {}
                }
            }
            for translated in translation_rx.try_iter() {
                if translated.session_id != session_id {
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
                        let status = type_voice_result(&candidate.translated, &config);
                        if translated.utterance_id == latest_utterance {
                            show_voice(
                                &app,
                                VoiceDisplay {
                                    original: translated.recognized,
                                    result: candidate.translated,
                                    status,
                                },
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
            if hide_at.is_some_and(|at| Instant::now() >= at) {
                hide_voice(&app);
                hide_at = None;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    });
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

fn open_settings(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.show();
        let _ = window.set_focus();
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
    let reload = MenuItem::with_id(app, "reload", "重新加载词库", true, None::<&str>)?;
    let exit = MenuItem::with_id(app, "exit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&query, &settings, &reload, &exit])?;
    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("程序图标缺失"))?;
    TrayIconBuilder::new()
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "query" => open_query(app),
            "settings" => open_settings(app),
            "reload" => {
                app.state::<WebState>()
                    .search
                    .lock()
                    .map(|mut value| value.library.clear())
                    .ok();
            }
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
                    Some("settings") => open_settings(&app),
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
        "settings" => open_settings(app),
        "ocr" => {
            let _ = open_ocr(app);
        }
        "voice" => {
            position_voice(app);
            let _ = toggle_voice(app.state::<WebState>());
        }
        _ => {}
    }
}

pub(crate) fn run() -> anyhow::Result<()> {
    let config = load_ui_config()?;
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
            voice_cache: Mutex::new(None),
            voice_display: Mutex::new(VoiceDisplay::default()),
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
            recognize_ocr,
            confirm_ocr,
            cancel_ocr,
            get_voice_display,
            take_pending_query,
            toggle_voice,
            review_voice_cache_now,
            voice_registered,
            delete_voice_profile,
            register_voice_profile,
        ])
        .setup(|app| {
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
            let (voice_tx, voice_rx) = mpsc::channel();
            let (cache_tx, cache_rx) = mpsc::channel();
            *app.state::<WebState>()
                .voice_control
                .lock()
                .map_err(error)? = Some(voice_tx);
            *app.state::<WebState>().voice_cache.lock().map_err(error)? = Some(cache_tx);
            start_cache_worker(app.handle().clone(), cache_rx);
            start_voice_worker(app.handle().clone(), voice_rx);
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
                if let Some(window) = app.get_webview_window(&label) {
                    let _ = window.hide();
                }
            }
        }
        let _ = &manager;
    });
    Ok(())
}
