//! OCR 状态、截图、识别与截图浮层命令。

use super::*;

#[tauri::command]
pub(super) fn ocr_engine_status(state: State<'_, WebState>) -> String {
    if let Ok(config) = state.config.lock() {
        crate::i18n::set_language(&config.ui_language);
    }
    ocr::engine_status()
}

#[tauri::command]
pub(super) fn get_ocr_snapshot(state: State<'_, WebState>) -> Result<OcrSnapshot, String> {
    use base64::Engine;
    use image::{ColorType, ImageEncoder, codecs::png::PngEncoder};
    let generation = state.ocr_generation.load(Ordering::Relaxed);
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
    if state.ocr_generation.load(Ordering::Relaxed) != generation {
        return Err("截图已更新，请稍候".to_string());
    }
    Ok(OcrSnapshot {
        image: format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(png)
        ),
        generation,
    })
}

#[derive(Serialize)]
pub(super) struct OcrSnapshot {
    image: String,
    generation: u64,
}

#[tauri::command]
pub(super) fn show_ocr(generation: u64, app: AppHandle) -> Result<bool, String> {
    // 较早的一张截图完成解码时，不能把过期画面重新显示出来。
    if app
        .state::<WebState>()
        .ocr_generation
        .load(Ordering::Relaxed)
        != generation
    {
        return Ok(false);
    }
    let window = app.get_webview_window("ocr").ok_or("OCR 窗口不存在")?;
    window.show().map_err(error)?;
    window.set_focus().map_err(error)?;
    Ok(true)
}

#[tauri::command]
pub(super) async fn recognize_ocr(
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
pub(super) fn confirm_ocr(app: AppHandle) -> Result<(), String> {
    let original = app
        .state::<WebState>()
        .ocr_original
        .lock()
        .map_err(error)?
        .clone()
        .ok_or("尚未识别到文字")?;
    write_clipboard(original)?;
    close_ocr(&app)
}

#[tauri::command]
pub(super) fn cancel_ocr(app: AppHandle) -> Result<(), String> {
    close_ocr(&app)
}

fn close_ocr(app: &AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("ocr") {
        window.hide().map_err(error)?;
    }
    let state = app.state::<WebState>();
    state.ocr_generation.fetch_add(1, Ordering::Relaxed);
    *state.ocr_screen.lock().map_err(error)? = None;
    *state.ocr_original.lock().map_err(error)? = None;
    Ok(())
}

pub(super) fn open_ocr(app: &AppHandle) -> Result<(), String> {
    use tauri::Emitter;
    // 截屏先于覆盖层显示，避免把覆盖层自己拍进去。
    let monitor = ocr::monitor_under_cursor().map_err(error)?;
    let screen = Arc::new(ocr::capture_monitor(monitor).map_err(error)?);
    let generation = app
        .state::<WebState>()
        .ocr_generation
        .fetch_add(1, Ordering::Relaxed)
        + 1;
    *app.state::<WebState>().ocr_screen.lock().map_err(error)? = Some(screen);
    *app.state::<WebState>().ocr_original.lock().map_err(error)? = None;
    let window = if let Some(window) = app.get_webview_window("ocr") {
        window
    } else {
        create_ocr_window(app).map_err(error)?
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
    // 隐藏窗口中的 React 页面先解码图片，随后通过 show_ocr 再显示。
    app.emit_to("ocr", "ocr-reset", generation).map_err(error)?;
    Ok(())
}

pub(super) fn create_ocr_window(app: &AppHandle) -> tauri::Result<tauri::WebviewWindow> {
    WebviewWindowBuilder::new(app, "ocr", WebviewUrl::App("index.html?view=ocr".into()))
        .title("to_words OCR")
        .decorations(false)
        .shadow(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build()
}
