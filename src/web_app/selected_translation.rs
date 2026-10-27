//! Translate text selected in another Windows application. UI Automation reads
//! the selection; the optional right-click action is our own small window,
//! because other applications own their native text context menus.

use super::*;

const MENU_LIFETIME: Duration = Duration::from_secs(5);

pub(super) fn start_translation(app: &AppHandle, text: String) {
    let state = app.state::<WebState>();
    state
        .selection_menu_generation
        .fetch_add(1, Ordering::Relaxed);
    if let Ok(mut pending) = state.pending_selected_translation.lock() {
        *pending = Some(text);
    }
    if let Some(window) = app.get_webview_window("selection_menu") {
        let _ = window.hide();
    }
    open_query(app);
}

pub(super) fn translate_selected_hotkey(app: &AppHandle) {
    if !SELECTED_TEXT_TRANSLATION_ACTIVE {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let selected = selected_text::read_selected_text();
        let callback_app = app.clone();
        let _ = app.run_on_main_thread(move || match selected {
            Some(text) => start_translation(&callback_app, text),
            None => {
                use tauri::Emitter;
                open_query(&callback_app);
                let _ = callback_app.emit_to(
                    "query",
                    "selected-text-error",
                    "未读取到选中文字；当前程序可能未提供可读取的文字选区",
                );
            }
        });
    });
}

pub(super) fn offer_selected_text_menu(app: &AppHandle) {
    if !SELECTED_TEXT_TRANSLATION_ACTIVE {
        return;
    }
    let state = app.state::<WebState>();
    let generation = state
        .selection_menu_generation
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    if let Ok(mut slot) = state.selection_menu_text.lock() {
        *slot = None;
    }
    if let Some(window) = app.get_webview_window("selection_menu") {
        let _ = window.hide();
    }
    let cursor = selected_text::cursor_position();
    let app = app.clone();
    std::thread::spawn(move || {
        let Some(text) = selected_text::read_selected_text() else {
            return;
        };
        let state = app.state::<WebState>();
        if state.selection_menu_generation.load(Ordering::Relaxed) != generation
            || !state
                .config
                .lock()
                .ok()
                .is_some_and(|config| config.selected_text_context_menu)
        {
            return;
        }
        if let Ok(mut slot) = state.selection_menu_text.lock() {
            *slot = Some(text);
        }
        let show_app = app.clone();
        let _ = app.run_on_main_thread(move || {
            if show_app
                .state::<WebState>()
                .selection_menu_generation
                .load(Ordering::Relaxed)
                != generation
            {
                return;
            }
            if let Some(window) = show_app.get_webview_window("selection_menu") {
                if let Some((cursor_x, cursor_y)) = cursor {
                    let (mut x, mut y) = (cursor_x + 18, cursor_y + 18);
                    if let (Ok(monitors), Ok(size)) =
                        (window.available_monitors(), window.outer_size())
                        && let Some(monitor) = monitors.into_iter().find(|monitor| {
                            let position = monitor.position();
                            let bounds = monitor.size();
                            cursor_x >= position.x
                                && cursor_x < position.x + bounds.width as i32
                                && cursor_y >= position.y
                                && cursor_y < position.y + bounds.height as i32
                        })
                    {
                        let position = monitor.position();
                        let bounds = monitor.size();
                        x = x.clamp(
                            position.x,
                            (position.x + bounds.width as i32 - size.width as i32).max(position.x),
                        );
                        y = y.clamp(
                            position.y,
                            (position.y + bounds.height as i32 - size.height as i32)
                                .max(position.y),
                        );
                    }
                    let _ = window.set_position(tauri::Position::Physical(
                        tauri::PhysicalPosition::new(x, y),
                    ));
                }
                let _ = window.show();
            }
        });
        std::thread::sleep(MENU_LIFETIME);
        if app
            .state::<WebState>()
            .selection_menu_generation
            .load(Ordering::Relaxed)
            == generation
        {
            let hide_app = app.clone();
            let _ = app.run_on_main_thread(move || hide_menu(&hide_app));
        }
    });
}

fn hide_menu(app: &AppHandle) {
    let state = app.state::<WebState>();
    state
        .selection_menu_generation
        .fetch_add(1, Ordering::Relaxed);
    if let Ok(mut slot) = state.selection_menu_text.lock() {
        *slot = None;
    }
    if let Some(window) = app.get_webview_window("selection_menu") {
        let _ = window.hide();
    }
}

#[tauri::command]
pub(super) fn take_pending_selected_translation(
    state: State<'_, WebState>,
) -> Result<Option<String>, String> {
    Ok(state
        .pending_selected_translation
        .lock()
        .map_err(error)?
        .take())
}

#[tauri::command]
pub(super) fn translate_menu_selection(app: AppHandle) -> Result<(), String> {
    if !SELECTED_TEXT_TRANSLATION_ACTIVE {
        return Err("选中文字翻译已暂时停用".into());
    }
    let text = app
        .state::<WebState>()
        .selection_menu_text
        .lock()
        .map_err(error)?
        .take()
        .ok_or("选中文字已过期，请重新选择")?;
    start_translation(&app, text);
    Ok(())
}

#[tauri::command]
pub(super) fn hide_selection_menu(app: AppHandle) {
    hide_menu(&app);
}

#[tauri::command]
pub(super) async fn translate_selected_text(
    text: String,
    state: State<'_, WebState>,
) -> Result<WebTranslation, String> {
    let text = text.trim().to_owned();
    if text.is_empty() || text.chars().count() > 4_000 {
        return Err("请选择不超过 4000 字的文字".into());
    }
    let config = state.config.lock().map_err(error)?.clone();
    if config.translation_language.is_empty() {
        return Err("请先在设置中选择目标语言".into());
    }
    let source = if config.source_language == "auto" {
        "cn"
    } else {
        config.source_language.as_str()
    };
    if source == config.translation_language {
        return Ok(WebTranslation {
            translated: text,
            pronunciation: String::new(),
            save_error: None,
        });
    }
    let key = credentials::load_key()
        .map_err(error)?
        .ok_or("请先在设置中填写 DeepSeek API Key")?;
    let source = source.to_owned();
    let target = config.translation_language;
    let politeness = config.ai_politeness;
    tauri::async_runtime::spawn_blocking(move || {
        let (translated, pronunciation) =
            ai::translate_selected_text(&text, &source, &target, &key, politeness)
                .map_err(error)?;
        Ok::<_, String>(WebTranslation {
            translated,
            pronunciation,
            save_error: None,
        })
    })
    .await
    .map_err(error)?
}
