//! 应用运行时核心。
//!
//! 本模块包含配置模型及读写、词库加载、快捷键注册、查询窗口定位、`MatchApp`
//! 状态和 eframe 生命周期实现。文件后半部分负责主查询框绘制、键盘交互、字体与
//! 样式初始化，`run` 是交给程序入口调用的启动函数。

use anyhow::{Context, Result, bail};
use eframe::egui;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState, hotkey::HotKey};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

const INPUT_ID: &str = "word_match_input";
const VOICE_OVERLAY_HOLD: Duration = Duration::from_secs(1);
pub(crate) const WORD_EDITOR_FONT_FAMILY: &str = "word_editor_chinese";

use crate::ai::{self, AiCandidate, TranslationDirection};
use crate::components::ocr_selection::{OcrSelection, SelectionAction};
use crate::components::settings::{
    AiKeyUpdate, SettingsPanel, shortcut_pressed, shortcut_released,
};
use crate::components::voice_overlay::VoiceOverlay;
use crate::domain::ai_word_save;
use crate::domain::continuous_input::ContinuousInput;
use crate::domain::shortcut::MouseButton;
use crate::domain::shortcut::Shortcut;
use crate::domain::storage;
use crate::domain::voice_word_cache::{self, VoiceCacheRequest, VoiceWordCache};
use crate::domain::word_index::{SearchHit, WordEntry, WordIndex};
use crate::platform::credentials;
use crate::platform::extra_mouse_buttons::ExtraMouseButtons;
use crate::platform::mouse_hotkey::MouseHotkeyListener;
use crate::platform::ocr::{self, CaptureRegion, CapturedScreen, MonitorBounds};
use crate::platform::system_tray::{SystemTray, TrayAction};
use crate::platform::voice::{self, VoiceEvent, VoiceSession};
use crate::platform::voice_metrics::VoiceTrace;
use crate::platform::voice_output::{VoiceInputMethod, VoiceInputTarget, input_error_trace_code};
use crate::{translation_language, ui_theme};

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
    #[serde(alias = "ai_korean_honorific")]
    pub(crate) ai_polite_mode: bool,
    pub(crate) ocr_auto_translate: bool,
    pub(crate) voice_auto_copy_first: bool,
    pub(crate) voice_aion2_manual_paste: bool,
    pub(crate) voice_keep_input: bool,
    pub(crate) voice_hold_to_talk: bool,
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
            ai_polite_mode: false,
            ocr_auto_translate: true,
            voice_auto_copy_first: true,
            voice_aion2_manual_paste: false,
            voice_keep_input: false,
            voice_hold_to_talk: false,
            voice_silence_seconds: 0.6,
            voice_backend: "sensevoice".to_string(),
            voice_language: "auto".to_string(),
            clear_on_focus_return: false,
            translation_language: String::new(),
            source_language: "auto".to_string(),
            ui_language: "zh-CN".to_string(),
            text_color: "#24272E".to_string(),
            selected_color: "#A9CEFF".to_string(),
            selected_text_color: "#0B57D0".to_string(),
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
            background_color: "#E8E8E8".to_string(),
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
            background_color: "#E8E8E8".to_string(),
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
            popup: "ctrl+alt+enter".to_string(),
            settings: "ctrl+alt+s".to_string(),
            clear_composed: "ctrl+backspace".to_string(),
            ocr: "ctrl+alt+o".to_string(),
            voice: "ctrl+alt+v".to_string(),
        }
    }
}

impl UiConfig {
    fn normalized(mut self) -> Self {
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
        self.voice_silence_seconds = self.voice_silence_seconds.clamp(0.2, 3.0);
        self.voice_silence_seconds = (self.voice_silence_seconds * 10.0).round() / 10.0;
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
            self.voice_backend = "sensevoice".to_string();
        }
        self
    }

    fn input_color(&self) -> egui::Color32 {
        parse_hex_color(&self.input.background_color, self.input.opacity)
            .unwrap_or_else(|_| color_with_opacity(232, 232, 232, 0.20))
    }

    fn results_color(&self) -> egui::Color32 {
        parse_hex_color(&self.results.background_color, self.results.opacity)
            .unwrap_or_else(|_| color_with_opacity(232, 232, 232, 0.20))
    }

    fn text_color(&self) -> egui::Color32 {
        parse_hex_color(&self.text_color, 1.0)
            .unwrap_or_else(|_| egui::Color32::from_rgb(36, 39, 46))
    }

    fn selected_color(&self) -> egui::Color32 {
        parse_hex_color(&self.selected_color, self.selected_opacity)
            .unwrap_or_else(|_| color_with_opacity(169, 206, 255, 0.72))
    }

    fn selected_text_color(&self) -> egui::Color32 {
        parse_hex_color(&self.selected_text_color, 1.0)
            .unwrap_or_else(|_| egui::Color32::from_rgb(11, 87, 208))
    }
}

pub(crate) fn project_directory() -> PathBuf {
    let working_directory = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if working_directory.join("ui_config.json").exists()
        || working_directory.join(storage::DATABASE_FILE).exists()
        || working_directory.join("user_words.json").exists()
        || working_directory.join("word_libraries").exists()
    {
        return working_directory;
    }

    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or(working_directory)
}

pub(crate) fn word_library_directory() -> PathBuf {
    project_directory().join("word_libraries")
}

fn load_user_words(config: &UiConfig, source: &str) -> Result<WordIndex> {
    let file_name = translation_language::word_file_name(source, &config.translation_language);
    Ok(WordIndex::from_map(
        storage::load_word_map(&file_name)?.into_iter().collect(),
    ))
}

fn load_ui_config() -> Result<UiConfig> {
    let Some(content) = storage::load_setting("ui_config")? else {
        return Ok(UiConfig::default());
    };
    let config: UiConfig = serde_json::from_str(&content).context("数据库中的 UI 设置格式错误")?;
    Ok(config.normalized())
}

fn save_ui_config(config: &UiConfig) -> Result<()> {
    let content = serde_json::to_string_pretty(config).context("无法生成 UI 配置")?;
    storage::save_setting("ui_config", &content)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HotkeyBindings {
    popup: Shortcut,
    settings: Shortcut,
    clear_composed: Shortcut,
    ocr: Shortcut,
    voice: Shortcut,
}

impl HotkeyBindings {
    fn global_keyboard_keys(self) -> Vec<HotKey> {
        [self.popup, self.settings, self.ocr, self.voice]
            .into_iter()
            .filter_map(Shortcut::keyboard)
            .collect()
    }

    fn uses_extra_mouse_button(self) -> bool {
        [
            self.popup,
            self.settings,
            self.clear_composed,
            self.ocr,
            self.voice,
        ]
        .into_iter()
        .filter_map(Shortcut::mouse)
        .any(|shortcut| {
            matches!(
                shortcut.button,
                MouseButton::Extra6 | MouseButton::Extra7 | MouseButton::Extra8
            )
        })
    }
}

fn parse_hotkeys(config: &UiConfig) -> Result<HotkeyBindings> {
    let parse = |label: &str, value: &str| {
        value
            .parse::<Shortcut>()
            .map_err(|error| anyhow::anyhow!("{label}快捷键格式无效：{value}：{error}"))
    };
    let popup = parse("查询框", &config.hotkeys.popup)?;
    let settings = parse("设置面板", &config.hotkeys.settings)?;
    let clear_composed = parse("清空组合内容", &config.hotkeys.clear_composed)?;
    let ocr = parse("OCR", &config.hotkeys.ocr)?;
    let voice = parse("语音识别", &config.hotkeys.voice)?;
    let bindings = HotkeyBindings {
        popup,
        settings,
        clear_composed,
        ocr,
        voice,
    };
    let all = [popup, settings, clear_composed, ocr, voice];
    for (index, shortcut) in all.iter().enumerate() {
        if all[index + 1..].contains(shortcut) {
            bail!("快捷键不能使用相同的组合");
        }
    }
    Ok(bindings)
}

fn register_global_keyboard_hotkeys(
    manager: &GlobalHotKeyManager,
    bindings: HotkeyBindings,
) -> Result<()> {
    let mut registered = Vec::new();
    for (label, binding) in [
        ("查询框", bindings.popup),
        ("设置面板", bindings.settings),
        ("OCR", bindings.ocr),
        ("语音识别", bindings.voice),
    ] {
        if let Some(key) = binding.keyboard() {
            if let Err(error) = manager.register(key) {
                let _ = manager.unregister_all(&registered);
                return Err(error)
                    .with_context(|| format!("{label}快捷键注册失败，可能已被其他程序占用"));
            }
            registered.push(key);
        }
    }
    Ok(())
}

fn parse_hex_color(color: &str, opacity: f32) -> Result<egui::Color32> {
    let hex = color.trim().trim_start_matches('#');
    if hex.len() != 6 {
        bail!("颜色必须采用 #RRGGBB 格式：{color}");
    }
    let red = u8::from_str_radix(&hex[0..2], 16)?;
    let green = u8::from_str_radix(&hex[2..4], 16)?;
    let blue = u8::from_str_radix(&hex[4..6], 16)?;
    Ok(color_with_opacity(red, green, blue, opacity))
}

fn color_with_opacity(red: u8, green: u8, blue: u8, opacity: f32) -> egui::Color32 {
    egui::Color32::from_rgba_unmultiplied(
        red,
        green,
        blue,
        (opacity.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

#[derive(Clone, Copy, Debug)]
struct PopupAnchor {
    x: f32,
    caret_rect: Option<egui::Rect>,
    fallback_bottom: f32,
    work_rect: egui::Rect,
}

#[cfg(target_os = "windows")]
fn capture_popup_anchor(pixels_per_point: f32) -> Option<PopupAnchor> {
    use windows_sys::Win32::{
        Foundation::POINT,
        Graphics::Gdi::{
            GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL, MONITORINFO,
            MonitorFromPoint, MonitorFromWindow,
        },
        UI::WindowsAndMessaging::{GetCursorPos, GetForegroundWindow},
    };

    unsafe {
        let foreground = GetForegroundWindow();
        let scale = pixels_per_point.max(1.0);
        let caret_rect = capture_system_caret_rect(scale)
            .or_else(|| capture_uia_caret_rect(scale))
            .or_else(|| capture_msaa_caret_rect(scale))
            .or_else(|| capture_ime_caret_rect(scale));
        let mut monitor = if let Some(caret) = caret_rect {
            MonitorFromPoint(
                POINT {
                    x: (caret.left() * scale).round() as i32,
                    y: (caret.bottom() * scale).round() as i32,
                },
                MONITOR_DEFAULTTONEAREST,
            )
        } else if foreground.is_null() {
            std::ptr::null_mut()
        } else {
            MonitorFromWindow(foreground, MONITOR_DEFAULTTONULL)
        };
        if monitor.is_null() {
            let mut cursor = POINT { x: 0, y: 0 };
            if GetCursorPos(&mut cursor) == 0 {
                return None;
            }
            monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
        }
        if monitor.is_null() {
            return None;
        }

        let mut monitor_info: MONITORINFO = std::mem::zeroed();
        monitor_info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut monitor_info) == 0 {
            return None;
        }

        let work_area = monitor_info.rcWork;
        let work_rect = egui::Rect::from_min_max(
            egui::pos2(work_area.left as f32 / scale, work_area.top as f32 / scale),
            egui::pos2(
                work_area.right as f32 / scale,
                work_area.bottom as f32 / scale,
            ),
        );
        Some(PopupAnchor {
            x: caret_rect.map_or(work_rect.left() + 12.0, |caret| caret.left()),
            caret_rect,
            fallback_bottom: work_rect.bottom() - 52.0,
            work_rect,
        })
    }
}

#[cfg(target_os = "windows")]
unsafe fn capture_system_caret_rect(scale: f32) -> Option<egui::Rect> {
    use windows_sys::Win32::{
        Foundation::POINT,
        Graphics::Gdi::ClientToScreen,
        UI::WindowsAndMessaging::{GUITHREADINFO, GetGUIThreadInfo},
    };

    let mut info: GUITHREADINFO = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
    if unsafe { GetGUIThreadInfo(0, &mut info) } == 0 || info.hwndCaret.is_null() {
        return None;
    }

    let mut top_left = POINT {
        x: info.rcCaret.left,
        y: info.rcCaret.top,
    };
    let mut bottom_right = POINT {
        x: info.rcCaret.right,
        y: info.rcCaret.bottom,
    };
    if unsafe { ClientToScreen(info.hwndCaret, &mut top_left) } == 0
        || unsafe { ClientToScreen(info.hwndCaret, &mut bottom_right) } == 0
    {
        return None;
    }

    Some(egui::Rect::from_min_max(
        egui::pos2(top_left.x as f32 / scale, top_left.y as f32 / scale),
        egui::pos2(bottom_right.x as f32 / scale, bottom_right.y as f32 / scale),
    ))
}

#[cfg(target_os = "windows")]
unsafe fn capture_uia_caret_rect(scale: f32) -> Option<egui::Rect> {
    use windows::Win32::{
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize,
        },
        UI::Accessibility::{CUIAutomation, IUIAutomation},
    };

    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    let result = (|| {
        let automation: IUIAutomation =
            unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.ok()?;
        let focused = unsafe { automation.GetFocusedElement() }.ok()?;
        let walker = unsafe { automation.RawViewWalker() }.ok()?;
        let mut current = Some(focused);
        for _ in 0..10 {
            let element = current.take()?;
            if let Some(rect) = unsafe { uia_element_caret_rect(&element, scale) } {
                return Some(rect);
            }
            current = unsafe { walker.GetParentElement(&element) }.ok();
        }
        None
    })();
    if initialized {
        unsafe { CoUninitialize() };
    }
    result
}

#[cfg(target_os = "windows")]
unsafe fn uia_element_caret_rect(
    element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
    scale: f32,
) -> Option<egui::Rect> {
    use windows::Win32::{
        Foundation::BOOL,
        UI::Accessibility::{
            IUIAutomationTextPattern, IUIAutomationTextPattern2, UIA_TextPattern2Id,
            UIA_TextPatternId,
        },
    };

    let caret_range = unsafe {
        element
            .GetCurrentPatternAs::<IUIAutomationTextPattern2>(UIA_TextPattern2Id)
            .ok()
            .and_then(|pattern| {
                let mut active = BOOL(0);
                pattern.GetCaretRange(&mut active).ok()
            })
    };
    if let Some(rect) = caret_range
        .as_ref()
        .and_then(|range| unsafe { uia_range_rect(range, scale) })
    {
        return Some(rect);
    }

    let selection_range = unsafe {
        let pattern = element
            .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
            .ok()?;
        let selection = pattern.GetSelection().ok()?;
        (selection.Length().ok()? > 0)
            .then(|| selection.GetElement(0).ok())
            .flatten()
    }?;
    unsafe { uia_range_rect(&selection_range, scale) }
}

#[cfg(target_os = "windows")]
unsafe fn uia_range_rect(
    range: &windows::Win32::UI::Accessibility::IUIAutomationTextRange,
    scale: f32,
) -> Option<egui::Rect> {
    use windows::Win32::System::Ole::SafeArrayDestroy;

    let rectangles = unsafe { range.GetBoundingRectangles() }.ok()?;
    if rectangles.is_null() {
        return None;
    }
    let result = unsafe { last_uia_rectangle(rectangles, scale) };
    let _ = unsafe { SafeArrayDestroy(rectangles) };
    result
}

#[cfg(target_os = "windows")]
unsafe fn last_uia_rectangle(
    rectangles: *mut windows::Win32::System::Com::SAFEARRAY,
    scale: f32,
) -> Option<egui::Rect> {
    use windows::Win32::System::Ole::{
        SafeArrayGetDim, SafeArrayGetElement, SafeArrayGetLBound, SafeArrayGetUBound,
    };

    if unsafe { SafeArrayGetDim(rectangles) } != 1 {
        return None;
    }
    let lower = unsafe { SafeArrayGetLBound(rectangles, 1) }.ok()?;
    let upper = unsafe { SafeArrayGetUBound(rectangles, 1) }.ok()?;
    let count = upper.checked_sub(lower)?.checked_add(1)?;
    if count < 4 || count % 4 != 0 {
        return None;
    }
    let start = upper - 3;
    let mut values = [0.0_f64; 4];
    for (offset, value) in values.iter_mut().enumerate() {
        let index = start + offset as i32;
        unsafe { SafeArrayGetElement(rectangles, &index, std::ptr::from_mut(value).cast()) }
            .ok()?;
    }
    let [left, top, width, height] = values;
    if !values.iter().all(|value| value.is_finite()) || height <= 0.0 {
        return None;
    }
    let scale = f64::from(scale.max(1.0));
    Some(egui::Rect::from_min_max(
        egui::pos2((left / scale) as f32, (top / scale) as f32),
        egui::pos2(
            ((left + width.max(1.0)) / scale) as f32,
            ((top + height) / scale) as f32,
        ),
    ))
}

#[cfg(target_os = "windows")]
unsafe fn capture_msaa_caret_rect(scale: f32) -> Option<egui::Rect> {
    use windows::{
        Win32::{
            Foundation::HWND,
            System::{
                Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
                Variant::VARIANT,
            },
            UI::Accessibility::{AccessibleObjectFromWindow, IAccessible},
        },
        core::Interface,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::OBJID_CARET;

    let focused = unsafe { focused_window_handle() }?;
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    let result = (|| {
        let mut raw = std::ptr::null_mut();
        unsafe {
            AccessibleObjectFromWindow(
                HWND(focused),
                OBJID_CARET as u32,
                &IAccessible::IID,
                &mut raw,
            )
        }
        .ok()?;
        if raw.is_null() {
            return None;
        }
        let accessible = unsafe { IAccessible::from_raw(raw) };
        let child = VARIANT::from(0_i32);
        let (mut left, mut top, mut width, mut height) = (0, 0, 0, 0);
        unsafe { accessible.accLocation(&mut left, &mut top, &mut width, &mut height, &child) }
            .ok()?;
        if height <= 0 {
            return None;
        }
        let scale = scale.max(1.0);
        Some(egui::Rect::from_min_max(
            egui::pos2(left as f32 / scale, top as f32 / scale),
            egui::pos2(
                (left + width.max(1)) as f32 / scale,
                (top + height) as f32 / scale,
            ),
        ))
    })();
    if initialized {
        unsafe { CoUninitialize() };
    }
    result
}

#[cfg(target_os = "windows")]
unsafe fn capture_ime_caret_rect(scale: f32) -> Option<egui::Rect> {
    use windows_sys::Win32::{
        Foundation::POINT,
        Graphics::Gdi::ClientToScreen,
        UI::Input::Ime::{
            CANDIDATEFORM, CFS_CANDIDATEPOS, CFS_EXCLUDE, CFS_FORCE_POSITION, CFS_POINT, CFS_RECT,
            COMPOSITIONFORM, ImmGetCandidateWindow, ImmGetCompositionWindow, ImmGetContext,
            ImmReleaseContext,
        },
    };

    let focused = unsafe { focused_window_handle() }?;
    let input_context = unsafe { ImmGetContext(focused) };
    if input_context.is_null() {
        return None;
    }

    let mut point = None;
    let mut composition: COMPOSITIONFORM = unsafe { std::mem::zeroed() };
    if unsafe { ImmGetCompositionWindow(input_context, &mut composition) } != 0
        && matches!(
            composition.dwStyle,
            CFS_POINT | CFS_FORCE_POSITION | CFS_RECT
        )
    {
        point = Some(composition.ptCurrentPos);
    }
    if point.is_none() {
        let mut candidate: CANDIDATEFORM = unsafe { std::mem::zeroed() };
        if unsafe { ImmGetCandidateWindow(input_context, 0, &mut candidate) } != 0
            && matches!(candidate.dwStyle, CFS_CANDIDATEPOS | CFS_EXCLUDE)
        {
            point = Some(candidate.ptCurrentPos);
        }
    }
    unsafe { ImmReleaseContext(focused, input_context) };

    let mut point: POINT = point?;
    if unsafe { ClientToScreen(focused, &mut point) } == 0 {
        return None;
    }
    let scale = scale.max(1.0);
    let left = point.x as f32 / scale;
    let top = point.y as f32 / scale;
    Some(egui::Rect::from_min_max(
        egui::pos2(left, top),
        egui::pos2(left + 2.0, top + 20.0),
    ))
}

#[cfg(target_os = "windows")]
unsafe fn focused_window_handle() -> Option<windows_sys::Win32::Foundation::HWND> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GUITHREADINFO, GetGUIThreadInfo};

    let mut info: GUITHREADINFO = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
    if unsafe { GetGUIThreadInfo(0, &mut info) } == 0 || info.hwndFocus.is_null() {
        None
    } else {
        Some(info.hwndFocus)
    }
}

#[cfg(not(target_os = "windows"))]
fn capture_popup_anchor(_pixels_per_point: f32) -> Option<PopupAnchor> {
    None
}

fn popup_position(anchor: PopupAnchor, viewport_size: egui::Vec2) -> egui::Pos2 {
    let max_x = (anchor.work_rect.right() - viewport_size.x).max(anchor.work_rect.left());
    let x = anchor.x.clamp(anchor.work_rect.left(), max_x);
    let proposed_y = if let Some(caret) = anchor.caret_rect {
        let below = caret.bottom() + 8.0;
        if below + viewport_size.y <= anchor.work_rect.bottom() {
            below
        } else {
            caret.top() - 8.0 - viewport_size.y
        }
    } else {
        anchor.fallback_bottom - viewport_size.y
    };
    let max_y = (anchor.work_rect.bottom() - viewport_size.y).max(anchor.work_rect.top());
    egui::pos2(x, proposed_y.clamp(anchor.work_rect.top(), max_y))
}

enum AiState {
    Idle,
    Loading(Receiver<Result<AiCandidate>>),
    Ready(AiCandidate),
    Error,
}

enum OcrState {
    Idle,
    Preparing {
        monitor: MonitorBounds,
        ready_at: Instant,
    },
    Capturing(Receiver<Result<CapturedScreen>>),
    Selecting {
        selection: OcrSelection,
        preview: OcrPreview,
    },
    Finalizing(Receiver<Result<String>>),
}

#[derive(Default)]
struct OcrPreview {
    region: Option<CaptureRegion>,
    ready_at: Option<Instant>,
    recognizing: Option<Receiver<Result<String>>>,
    recognized: Option<String>,
    translating: Option<Receiver<Result<String>>>,
    display: Option<(String, bool)>,
}

enum VoiceSaveRequest {
    Candidate(VoiceCacheRequest),
    ReviewNow,
}

struct VoiceSaveEvent {
    library_name: String,
    saved: Result<()>,
}

fn start_voice_save_worker() -> (mpsc::Sender<VoiceSaveRequest>, Receiver<VoiceSaveEvent>) {
    let (request_sender, request_receiver) = mpsc::channel::<VoiceSaveRequest>();
    let (event_sender, event_receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let path = storage::database_path();
        let mut cache = match VoiceWordCache::load(path) {
            Ok(cache) => cache,
            Err(error) => {
                let _ = event_sender.send(VoiceSaveEvent {
                    library_name: String::new(),
                    saved: Err(error),
                });
                return;
            }
        };
        // 重新启动时也审核上次遗留的缓存；平时一小时或满 20 条触发。
        let mut last_review = if cache.pending_len() > 0 {
            Instant::now() - voice_word_cache::REVIEW_INTERVAL
        } else {
            Instant::now()
        };
        loop {
            let mut force_review = false;
            match request_receiver.recv_timeout(Duration::from_secs(30)) {
                Ok(VoiceSaveRequest::Candidate(request)) => {
                    if let Err(error) = cache.enqueue(request) {
                        let _ = event_sender.send(VoiceSaveEvent {
                            library_name: String::new(),
                            saved: Err(error),
                        });
                    }
                }
                Ok(VoiceSaveRequest::ReviewNow) => force_review = true,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if let Err(error) = cache.expire_uncertain() {
                let _ = event_sender.send(VoiceSaveEvent {
                    library_name: String::new(),
                    saved: Err(error),
                });
            }
            if !voice_review_due(
                cache.reviewable_len(),
                cache.pending_len(),
                force_review,
                last_review.elapsed(),
            ) {
                continue;
            }
            last_review = Instant::now();
            let key = match credentials::load_key() {
                Ok(Some(key)) if !key.trim().is_empty() => key,
                Ok(_) => {
                    if force_review {
                        let _ = event_sender.send(VoiceSaveEvent {
                            library_name: String::new(),
                            saved: Err(anyhow::anyhow!(
                                "未配置 DeepSeek API Key，语音缓存暂未审核"
                            )),
                        });
                    }
                    continue; // 没有 Key 时保留缓存，不丢弃待审核数据。
                }
                Err(error) => {
                    let _ = event_sender.send(VoiceSaveEvent {
                        library_name: String::new(),
                        saved: Err(error.context("读取语音词库审核用 API Key 失败")),
                    });
                    continue;
                }
            };
            // 单次定时任务最多审核 100 条；始终在后台线程运行。
            for _ in 0..10 {
                let before = cache.reviewable_len();
                if before == 0 {
                    break;
                }
                match cache.review_once(&key) {
                    Ok(saved) => {
                        let mut libraries = Vec::new();
                        for candidate in saved {
                            let name = translation_language::word_file_name(
                                &candidate.source_language_code,
                                &candidate.language_code,
                            );
                            if !libraries.contains(&name) {
                                libraries.push(name);
                            }
                        }
                        for library_name in libraries {
                            let _ = event_sender.send(VoiceSaveEvent {
                                library_name,
                                saved: Ok(()),
                            });
                        }
                        if cache.reviewable_len() >= before {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = event_sender.send(VoiceSaveEvent {
                            library_name: String::new(),
                            saved: Err(error),
                        });
                        break;
                    }
                }
            }
        }
    });
    (request_sender, event_receiver)
}

fn voice_review_due(
    reviewable: usize,
    pending: usize,
    force_review: bool,
    elapsed: Duration,
) -> bool {
    reviewable > 0
        && (force_review
            || pending >= voice_word_cache::REVIEW_THRESHOLD
            || elapsed >= voice_word_cache::REVIEW_INTERVAL)
}

impl OcrPreview {
    fn schedule(&mut self, region: CaptureRegion) {
        if self.region == Some(region) {
            return;
        }
        self.region = Some(region);
        self.ready_at = Some(Instant::now() + Duration::from_millis(300));
        self.recognizing = None;
        self.recognized = None;
        self.translating = None;
        self.display = None;
    }
}

fn translate_ocr_recognized(recognized: &str, target: &str, polite_mode: bool) -> Result<String> {
    let key = credentials::load_key()
        .context("读取 DeepSeek API Key 失败")?
        .filter(|key| !key.trim().is_empty())
        .context("未配置 DeepSeek API Key")?;
    ai::translate_ocr_text(recognized, target, &key, polite_mode)
}

fn same_language_no_translation(source: &str, target: &str) -> bool {
    !target.is_empty() && source != "auto" && source == target
}

fn ocr_translation_target(enabled: bool, source: &str, target: &str) -> Option<String> {
    (enabled && !same_language_no_translation(source, target)).then(|| {
        if source == "auto" {
            "cn".to_string()
        } else {
            source.to_string()
        }
    })
}

struct MatchApp {
    hotkey_manager: GlobalHotKeyManager,
    hotkeys: HotkeyBindings,
    mouse_hotkeys: Option<MouseHotkeyListener>,
    mouse_hotkey_error: Option<String>,
    extra_mouse_buttons: Option<ExtraMouseButtons>,
    extra_mouse_attempted: bool,
    mouse_bindings_active_at: Instant,
    hotkeys_suspended: bool,
    system_tray: SystemTray,
    config: UiConfig,
    words: WordIndex,
    active_source_language: String,
    word_load_error: Option<String>,
    settings_panel: SettingsPanel,
    continuous_input: ContinuousInput,
    input: String,
    matches: Vec<SearchHit>,
    ai_state: AiState,
    ocr_state: OcrState,
    voice_session: Option<VoiceSession>,
    voice_hold_active: bool,
    voice_overlay: Option<VoiceOverlay>,
    voice_translation: Option<(u64, Receiver<Result<AiCandidate>>)>,
    voice_pending: VecDeque<(u64, String)>,
    voice_trace: Option<VoiceTrace>,
    voice_save_requests: mpsc::Sender<VoiceSaveRequest>,
    voice_save_results: Receiver<VoiceSaveEvent>,
    voice_last_logged_drawn_original_id: u64,
    voice_active_utterance_id: u64,
    voice_session_id: u64,
    voice_context: VecDeque<(u64, String)>,
    last_ai_candidate: Option<AiCandidate>,
    last_query: String,
    selected: usize,
    result_page: usize,
    no_match_delete_armed: bool,
    request_input_focus: bool,
    message: Option<String>,
    viewport_size: egui::Vec2,
    popup_anchor: Option<PopupAnchor>,
    manually_moved: bool,
    is_open: bool,
    settings_open: bool,
    exit_requested: bool,
    copied_since_focus: bool,
    left_after_copy: bool,
}

impl MatchApp {
    fn new(context: &egui::Context) -> Result<Self> {
        configure_fonts(context);
        configure_style(context);

        let config = load_ui_config().unwrap_or_default();
        let hotkeys = parse_hotkeys(&config)?;
        let (mouse_hotkeys, mouse_hotkey_error) = match MouseHotkeyListener::new() {
            Ok(listener) => (Some(listener), None),
            Err(error) => (None, Some(format!("鼠标快捷键监听不可用：{error:#}"))),
        };
        let manager = GlobalHotKeyManager::new().context("无法初始化全局快捷键")?;
        register_global_keyboard_hotkeys(&manager, hotkeys)?;
        let system_tray = SystemTray::new()?;
        if let Some(error) = &mouse_hotkey_error {
            system_tray.set_status(error);
        }
        let (voice_save_requests, voice_save_results) = start_voice_save_worker();
        let active_source_language = if config.source_language == "auto" {
            "cn".to_string()
        } else {
            config.source_language.clone()
        };
        let (words, word_load_error) = match load_user_words(&config, &active_source_language) {
            Ok(words) => (words, None),
            Err(error) => (
                WordIndex::default(),
                Some(format!("读取词条失败：{error:#}")),
            ),
        };
        let viewport_size = egui::vec2(config.input.width, config.input.height);
        let settings_panel = SettingsPanel::new(config.clone());
        context.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        context.request_repaint_after(Duration::from_millis(40));

        Ok(Self {
            hotkey_manager: manager,
            hotkeys,
            mouse_hotkeys,
            mouse_hotkey_error,
            extra_mouse_buttons: None,
            extra_mouse_attempted: false,
            mouse_bindings_active_at: Instant::now(),
            hotkeys_suspended: false,
            system_tray,
            config,
            words,
            active_source_language,
            word_load_error,
            settings_panel,
            continuous_input: ContinuousInput::default(),
            input: String::new(),
            matches: Vec::new(),
            ai_state: AiState::Idle,
            ocr_state: OcrState::Idle,
            voice_session: None,
            voice_hold_active: false,
            voice_overlay: None,
            voice_translation: None,
            voice_pending: VecDeque::new(),
            voice_trace: None,
            voice_save_requests,
            voice_save_results,
            voice_last_logged_drawn_original_id: 0,
            voice_active_utterance_id: 0,
            voice_session_id: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_micros() as u64,
            voice_context: VecDeque::new(),
            last_ai_candidate: None,
            last_query: String::new(),
            selected: 0,
            result_page: 0,
            no_match_delete_armed: false,
            request_input_focus: false,
            message: None,
            viewport_size,
            popup_anchor: None,
            manually_moved: false,
            is_open: false,
            settings_open: false,
            exit_requested: false,
            copied_since_focus: false,
            left_after_copy: false,
        })
    }

    fn open_popup(&mut self, context: &egui::Context) {
        self.ocr_state = OcrState::Idle;
        let (config, config_error) = match load_ui_config() {
            Ok(config) => (config, None),
            Err(error) => {
                let message = Some(format!("UI 配置错误：{error:#}"));
                (UiConfig::default(), message)
            }
        };
        let source = if config.source_language == "auto" {
            self.active_source_language.clone()
        } else {
            config.source_language.clone()
        };
        if config.translation_language != self.config.translation_language
            || source != self.active_source_language
        {
            match load_user_words(&config, &source) {
                Ok(words) => {
                    self.words = words;
                    self.active_source_language = source;
                    self.word_load_error = None;
                }
                Err(error) => {
                    self.words = WordIndex::default();
                    self.word_load_error = Some(format!("读取词条失败：{error:#}"));
                }
            }
        }
        self.config = config;
        self.continuous_input.clear();
        self.message = config_error.or_else(|| self.word_load_error.clone());
        self.input.clear();
        self.matches.clear();
        self.ai_state = AiState::Idle;
        self.last_ai_candidate = None;
        self.last_query.clear();
        self.selected = 0;
        self.result_page = 0;
        self.no_match_delete_armed = false;
        self.copied_since_focus = false;
        self.left_after_copy = false;
        self.request_input_focus = true;
        self.is_open = true;
        self.settings_open = false;
        self.popup_anchor = capture_popup_anchor(context.pixels_per_point());
        self.manually_moved = false;
        self.resize_window(context);
        context.send_viewport_cmd(egui::ViewportCommand::Title(
            "hotkey_word_match".to_string(),
        ));
        context.send_viewport_cmd(egui::ViewportCommand::Transparent(true));
        context.send_viewport_cmd(egui::ViewportCommand::Decorations(false));
        context.send_viewport_cmd(egui::ViewportCommand::Resizable(false));
        context.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(1.0, 1.0)));
        context.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(egui::vec2(
            10_000.0, 10_000.0,
        )));
        context.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
            egui::WindowLevel::AlwaysOnTop,
        ));
        context.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        context.send_viewport_cmd(egui::ViewportCommand::Focus);
        context.request_repaint();
    }

    fn hide(&mut self, context: &egui::Context) {
        self.ocr_state = OcrState::Idle;
        self.is_open = false;
        self.settings_open = false;
        context.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    }

    fn open_settings(&mut self, context: &egui::Context) {
        self.ocr_state = OcrState::Idle;
        self.settings_panel
            .open(&self.config, &self.active_source_language);
        if let Some(error) = &self.mouse_hotkey_error {
            self.settings_panel.set_result(Err(error.clone()));
        }
        self.is_open = true;
        self.settings_open = true;

        let size = egui::vec2(800.0, 600.0);
        context.send_viewport_cmd(egui::ViewportCommand::Title("to_words 设置".to_string()));
        context.send_viewport_cmd(egui::ViewportCommand::Transparent(false));
        context.send_viewport_cmd(egui::ViewportCommand::Decorations(true));
        context.send_viewport_cmd(egui::ViewportCommand::Resizable(true));
        context.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(
            560.0, 420.0,
        )));
        context.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(egui::vec2(
            10_000.0, 10_000.0,
        )));
        context.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
            egui::WindowLevel::Normal,
        ));
        context.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        if let Some(anchor) = capture_popup_anchor(context.pixels_per_point()) {
            context.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                anchor.work_rect.center().x - size.x / 2.0,
                anchor.work_rect.center().y - size.y / 2.0,
            )));
        }
        context.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        context.send_viewport_cmd(egui::ViewportCommand::Focus);
        context.request_repaint();
    }

    fn begin_ocr_capture(&mut self, context: &egui::Context) {
        let monitor = match ocr::monitor_under_cursor() {
            Ok(monitor) => monitor,
            Err(error) => {
                self.show_ocr_error(context, format!("无法启动 OCR 框选：{error:#}"));
                return;
            }
        };
        // 先隐藏应用窗口，再截取原屏幕。透明全屏窗口在部分显卡驱动下会呈黑色。
        self.ocr_state = OcrState::Preparing {
            monitor,
            ready_at: Instant::now() + Duration::from_millis(220),
        };
        self.is_open = false;
        self.settings_open = false;
        context.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        context.request_repaint_after(Duration::from_millis(230));
    }

    fn show_ocr_selection(&mut self, context: &egui::Context, screen: CapturedScreen) {
        let monitor = screen.monitor;
        let scale = context.pixels_per_point().max(1.0);
        self.ocr_state = OcrState::Selecting {
            selection: OcrSelection::new(context, screen),
            preview: OcrPreview::default(),
        };
        self.is_open = true;
        context.send_viewport_cmd(egui::ViewportCommand::Title(
            "to_words OCR 框选".to_string(),
        ));
        context.send_viewport_cmd(egui::ViewportCommand::Transparent(false));
        context.send_viewport_cmd(egui::ViewportCommand::Decorations(false));
        context.send_viewport_cmd(egui::ViewportCommand::Resizable(false));
        context.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(egui::vec2(1.0, 1.0)));
        context.send_viewport_cmd(egui::ViewportCommand::MaxInnerSize(egui::vec2(
            20_000.0, 20_000.0,
        )));
        context.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
            monitor.x as f32 / scale,
            monitor.y as f32 / scale,
        )));
        context.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
            monitor.width as f32 / scale,
            monitor.height as f32 / scale,
        )));
        context.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
            egui::WindowLevel::AlwaysOnTop,
        ));
        context.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        context.send_viewport_cmd(egui::ViewportCommand::Focus);
        context.request_repaint();
    }

    fn poll_ocr(&mut self, context: &egui::Context) {
        let ready_monitor = match &self.ocr_state {
            OcrState::Preparing { monitor, ready_at } if Instant::now() >= *ready_at => {
                Some(*monitor)
            }
            _ => None,
        };
        if let Some(monitor) = ready_monitor {
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = sender.send(ocr::capture_monitor(monitor));
            });
            self.ocr_state = OcrState::Capturing(receiver);
        }

        let capture = match &self.ocr_state {
            OcrState::Capturing(receiver) => match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Disconnected) => {
                    Some(Err(anyhow::anyhow!("屏幕截图任务意外中断")))
                }
                Err(TryRecvError::Empty) => None,
            },
            _ => None,
        };
        if let Some(capture) = capture {
            match capture {
                Ok(screen) => self.show_ocr_selection(context, screen),
                Err(error) => {
                    self.ocr_state = OcrState::Idle;
                    self.show_ocr_error(context, format!("无法截取屏幕：{error:#}"));
                }
            }
        }

        self.poll_ocr_preview();

        let finalized = match &self.ocr_state {
            OcrState::Finalizing(receiver) => match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Disconnected) => Some(Err(anyhow::anyhow!("OCR 任务意外中断"))),
                Err(TryRecvError::Empty) => None,
            },
            _ => None,
        };
        if let Some(result) = finalized {
            self.ocr_state = OcrState::Idle;
            match result {
                Ok(text) => self.copy_ocr_original(context, &text),
                Err(error) => self.show_ocr_error(context, format!("OCR 失败：{error:#}")),
            }
        }
    }

    fn poll_ocr_preview(&mut self) {
        let OcrState::Selecting { selection, preview } = &mut self.ocr_state else {
            return;
        };

        if preview
            .ready_at
            .is_some_and(|ready_at| Instant::now() >= ready_at)
            && let Some(region) = preview.region
        {
            let screen = Arc::clone(&selection.screen);
            let source = self.config.source_language.clone();
            let target = self.config.translation_language.clone();
            let (sender, receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = sender.send(ocr::recognize_selection(&screen, region, &source, &target));
            });
            preview.ready_at = None;
            preview.recognizing = Some(receiver);
            preview.display = Some(("正在识别…".to_string(), false));
        }

        let recognized =
            preview
                .recognizing
                .as_ref()
                .and_then(|receiver| match receiver.try_recv() {
                    Ok(result) => Some(result),
                    Err(TryRecvError::Disconnected) => {
                        Some(Err(anyhow::anyhow!("OCR 任务意外中断")))
                    }
                    Err(TryRecvError::Empty) => None,
                });
        if let Some(result) = recognized {
            preview.recognizing = None;
            match result {
                Ok(text) => {
                    preview.recognized = Some(text.clone());
                    if let Some(target) = ocr_translation_target(
                        self.config.ocr_auto_translate,
                        &self.config.source_language,
                        &self.config.translation_language,
                    ) {
                        let (sender, receiver) = mpsc::channel();
                        let polite_mode = self.config.ai_polite_mode;
                        std::thread::spawn(move || {
                            let _ =
                                sender.send(translate_ocr_recognized(&text, &target, polite_mode));
                        });
                        preview.translating = Some(receiver);
                        preview.display = Some(("正在翻译…".to_string(), false));
                    } else {
                        preview.display = None;
                    }
                }
                Err(error) => {
                    preview.display = Some((format!("OCR 识别失败：{error:#}"), true));
                }
            }
        }

        let translated =
            preview
                .translating
                .as_ref()
                .and_then(|receiver| match receiver.try_recv() {
                    Ok(result) => Some(result),
                    Err(TryRecvError::Disconnected) => {
                        Some(Err(anyhow::anyhow!("翻译任务意外中断")))
                    }
                    Err(TryRecvError::Empty) => None,
                });
        if let Some(result) = translated {
            preview.translating = None;
            preview.display = Some(match result {
                Ok(text) => (text, false),
                Err(error) => (format!("翻译失败：{error:#}"), true),
            });
        }
    }

    fn copy_ocr_original(&mut self, context: &egui::Context, text: &str) {
        match arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text)) {
            Ok(()) => self
                .system_tray
                .set_status(&format!("OCR 原文已复制 {} 个字符", text.chars().count())),
            Err(error) => self.show_ocr_error(context, format!("OCR 已识别，但复制失败：{error}")),
        }
    }

    fn show_ocr_error(&mut self, context: &egui::Context, message: String) {
        self.system_tray.set_status(&message);
        self.open_popup(context);
        self.message = Some(message);
        self.resize_window(context);
    }

    fn voice_work_area(&self, context: &egui::Context) -> egui::Rect {
        if let Some(anchor) = capture_popup_anchor(context.pixels_per_point()) {
            return anchor.work_rect;
        }
        if let Ok(monitor) = ocr::monitor_under_cursor() {
            let scale = context.pixels_per_point().max(1.0);
            return egui::Rect::from_min_size(
                egui::pos2(monitor.x as f32 / scale, monitor.y as f32 / scale),
                egui::vec2(monitor.width as f32 / scale, monitor.height as f32 / scale),
            );
        }
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 720.0))
    }

    fn ensure_voice_overlay(&mut self, context: &egui::Context) -> &mut VoiceOverlay {
        if self.voice_overlay.is_none() {
            self.voice_overlay = Some(VoiceOverlay::new(self.voice_work_area(context)));
        }
        self.voice_overlay.as_mut().expect("voice overlay exists")
    }

    fn toggle_voice(&mut self, context: &egui::Context) {
        if let Some(session) = &mut self.voice_session {
            if self.voice_hold_active {
                return;
            }
            session.stop();
            session.trace.record(0, "manual_stop_requested", None, "ok");
            if let Some(overlay) = &mut self.voice_overlay {
                overlay.status = "已手动停止，正在处理最后一句…".to_string();
            }
            return;
        }
        if let Some((utterance_id, _)) = self.voice_translation.take()
            && let Some(trace) = &self.voice_trace
        {
            trace.finish(utterance_id, "cancelled_by_new_session");
        }
        if let Some(trace) = &self.voice_trace {
            for (utterance_id, _) in &self.voice_pending {
                trace.finish(*utterance_id, "cancelled_by_new_session");
            }
        }
        self.voice_pending.clear();
        self.voice_trace = None;
        self.voice_last_logged_drawn_original_id = 0;
        self.voice_active_utterance_id = 0;
        self.voice_session_id = self.voice_session_id.wrapping_add(1);
        self.voice_context.clear();
        let mut overlay = VoiceOverlay::new(self.voice_work_area(context));
        let hold_to_talk = self.config.voice_hold_to_talk;
        overlay.status = if hold_to_talk {
            "请按住快捷键说话，松开后停止录音".to_string()
        } else {
            format!(
                "请说话；停顿约 {:.1} 秒后自动识别",
                self.config.voice_silence_seconds
            )
        };
        match voice::start(
            self.config.voice_keep_input || hold_to_talk,
            hold_to_talk,
            &self.config.voice_language,
            &self.config.voice_backend,
            self.config.voice_silence_seconds,
        ) {
            Ok(session) => {
                self.voice_trace = Some(session.trace.clone());
                self.voice_session = Some(session);
                self.voice_hold_active = hold_to_talk;
                self.system_tray.set_status("语音识别正在录音");
            }
            Err(error) => {
                overlay.status = format!("无法开始语音识别：{error:#}");
                overlay.finish_after(VOICE_OVERLAY_HOLD);
                self.system_tray.set_status(&overlay.status);
            }
        }
        self.voice_overlay = Some(overlay);
        context.request_repaint();
    }

    fn stop_held_voice(&mut self) {
        if !self.voice_hold_active {
            return;
        }
        self.voice_hold_active = false;
        if let Some(session) = &mut self.voice_session {
            session.stop();
            session.trace.record(0, "hold_released", None, "ok");
            if let Some(overlay) = &mut self.voice_overlay {
                overlay.status = "已松开快捷键，正在处理最后一句…".to_string();
            }
        }
    }

    fn poll_voice(&mut self, context: &egui::Context) {
        let saved_events: Vec<_> = self.voice_save_results.try_iter().collect();
        for event in saved_events {
            match event.saved {
                Ok(()) => {
                    let active_library_name = translation_language::word_file_name(
                        &self.active_source_language,
                        &self.config.translation_language,
                    );
                    if event.library_name == active_library_name {
                        match load_user_words(&self.config, &self.active_source_language) {
                            Ok(words) => {
                                self.words = words;
                                self.word_load_error = None;
                                self.last_query.clear();
                                self.matches.clear();
                            }
                            Err(error) => self.system_tray.set_status(&format!(
                                "审核后的语音词条已保存，但重新加载词库失败：{error:#}"
                            )),
                        }
                    }
                    if let Err(error) = self
                        .settings_panel
                        .refresh_word_editor_after_external_save(&event.library_name)
                    {
                        self.system_tray.set_status(&format!(
                            "审核词条已保存，但词库编辑器刷新失败：{error:#}"
                        ));
                    }
                }
                Err(error) => {
                    self.settings_panel
                        .show_word_editor_review_error(&format!("{error:#}"));
                    self.system_tray
                        .set_status(&format!("语音词库缓存或审核失败：{error:#}"));
                }
            }
        }
        let mut events = Vec::new();
        let mut recognized_this_frame = false;
        if let Some(session) = &self.voice_session {
            events.extend(session.receiver.try_iter());
        }
        for event in events {
            match event {
                VoiceEvent::SpeechStarted(utterance_id) => {
                    if let Some(trace) = &self.voice_trace {
                        trace.record(utterance_id, "ui_speech_started", None, "ok");
                    }
                    self.voice_active_utterance_id = utterance_id;
                    self.ensure_voice_overlay(context).begin_utterance();
                }
                VoiceEvent::Recognizing(utterance_id) => {
                    if let Some(trace) = &self.voice_trace {
                        trace.record(utterance_id, "ui_vad_received", None, "ok");
                    }
                    if utterance_id == self.voice_active_utterance_id
                        && let Some(overlay) = &mut self.voice_overlay
                    {
                        overlay.status = "说话结束，正在识别…".to_string();
                    }
                }
                VoiceEvent::Utterance(utterance_id, result) => match result {
                    Ok(text) => {
                        if let Some(trace) = &self.voice_trace {
                            trace.record(utterance_id, "asr_ui_received", None, "ok");
                        }
                        if utterance_id == self.voice_active_utterance_id {
                            let overlay = self.ensure_voice_overlay(context);
                            overlay.original = text.trim().to_string();
                            overlay.original_utterance_id = utterance_id;
                            overlay.result.clear();
                            overlay.status = "识别完成，正在查询词库…".to_string();
                            overlay.keep_visible();
                        }
                        self.voice_pending.push_back((utterance_id, text));
                        recognized_this_frame = true;
                        context.request_repaint();
                    }
                    Err(error) => {
                        if let Some(trace) = &self.voice_trace {
                            trace.finish(utterance_id, "asr_error");
                        }
                        self.finish_voice_error_for(utterance_id, format!("语音识别失败：{error}"));
                    }
                },
                VoiceEvent::Stopped(error) => {
                    self.voice_session = None;
                    self.voice_hold_active = false;
                    if let Some(trace) = &self.voice_trace {
                        trace.record(
                            0,
                            "ui_session_stopped",
                            None,
                            if error.is_some() { "error" } else { "ok" },
                        );
                    }
                    if let Some(error) = error {
                        self.finish_voice_error(format!("语音录音失败：{error}"));
                    } else if self.voice_pending.is_empty() && self.voice_translation.is_none() {
                        let recognition_failed = self
                            .voice_overlay
                            .as_ref()
                            .is_some_and(|overlay| overlay.status.starts_with("语音识别失败："));
                        if recognition_failed {
                            continue;
                        }
                        if self
                            .voice_overlay
                            .as_ref()
                            .is_some_and(|overlay| overlay.original.is_empty())
                        {
                            self.finish_voice_error(
                                "没有检测到语音，请检查麦克风或重试".to_string(),
                            );
                        } else {
                            self.finish_voice_status("语音输入已结束");
                        }
                    }
                }
            }
        }
        let translation = self
            .voice_translation
            .as_ref()
            .and_then(|(utterance_id, receiver)| match receiver.try_recv() {
                Ok(result) => Some((*utterance_id, result)),
                Err(TryRecvError::Disconnected) => {
                    Some((*utterance_id, Err(anyhow::anyhow!("AI 翻译任务中断"))))
                }
                Err(TryRecvError::Empty) => None,
            });
        if let Some((utterance_id, result)) = translation {
            self.voice_translation = None;
            if let Some(trace) = &self.voice_trace {
                trace.record(
                    utterance_id,
                    "ai_ui_received",
                    None,
                    if result.is_ok() { "ok" } else { "error" },
                );
            }
            match result {
                Ok(candidate) => {
                    let translated = candidate.translated.clone();
                    let newer_utterance_pending = !self.voice_pending.is_empty()
                        || self.voice_active_utterance_id > utterance_id;
                    if !newer_utterance_pending && let Some(overlay) = &mut self.voice_overlay {
                        overlay.original = candidate.source.clone();
                        overlay.result = translated.clone();
                    }
                    if self
                        .voice_save_requests
                        .send(VoiceSaveRequest::Candidate(VoiceCacheRequest {
                            previous: self
                                .voice_context
                                .iter()
                                .position(|(id, _)| *id == utterance_id)
                                .and_then(|index| index.checked_sub(1))
                                .and_then(|index| self.voice_context.get(index))
                                .map(|(_, text)| text.clone()),
                            next: self
                                .voice_context
                                .iter()
                                .position(|(id, _)| *id == utterance_id)
                                .and_then(|index| self.voice_context.get(index + 1))
                                .map(|(_, text)| text.clone())
                                .or_else(|| {
                                    self.voice_pending.front().map(|(_, text)| text.clone())
                                }),
                            candidate,
                            session_id: self.voice_session_id,
                        }))
                        .is_err()
                    {
                        self.system_tray
                            .set_status("词库保存线程已停止，AI 译文未保存");
                    }
                    let input_started = Instant::now();
                    // 翻译完成时再选目标，允许持续录音期间切换到另一个输入窗口。
                    let typed = VoiceInputTarget::capture().map_or(Ok(None), |target| {
                        target.type_if_active(
                            &translated,
                            self.config.voice_aion2_manual_paste,
                            self.config.voice_hold_to_talk,
                        )
                    });
                    if let Some(trace) = &self.voice_trace {
                        trace.record(
                            utterance_id,
                            "text_input",
                            Some(input_started.elapsed()),
                            match &typed {
                                Ok(Some(VoiceInputMethod::Paste)) => "paste_sent",
                                Ok(Some(VoiceInputMethod::Unicode)) => "unicode_sent",
                                Ok(Some(VoiceInputMethod::ManualPaste)) => "manual_paste_ready",
                                #[cfg(feature = "tsf-notepad-prototype")]
                                Ok(Some(VoiceInputMethod::TextService)) => "text_service_queued",
                                Ok(None) => "skipped",
                                Err(error) => input_error_trace_code(error),
                            },
                        );
                        trace.finish(
                            utterance_id,
                            match &typed {
                                Ok(Some(_)) => "ai_input_sent",
                                Ok(None) => "input_skipped",
                                Err(_) => "input_error",
                            },
                        );
                    }
                    match typed {
                        Ok(Some(method)) if newer_utterance_pending => {
                            self.system_tray
                                .set_status(voice_input_status(true, method));
                        }
                        Ok(Some(method)) => {
                            self.finish_voice_status_for(
                                utterance_id,
                                voice_input_status(true, method),
                            );
                        }
                        Ok(None) if newer_utterance_pending => {
                            self.system_tray
                                .set_status("当前窗口不可用或焦点已变化，未输入 AI 译文");
                        }
                        Ok(None) => {
                            self.finish_voice_status_for(
                                utterance_id,
                                "当前窗口不可用或焦点已变化，未输入 AI 译文",
                            );
                        }
                        Err(reason) if newer_utterance_pending => {
                            self.system_tray
                                .set_status(&format!("AI 译文输入失败：{reason:#}"));
                        }
                        Err(reason) => {
                            self.finish_voice_status_for(
                                utterance_id,
                                &format!("AI 译文输入失败：{reason:#}"),
                            );
                        }
                    }
                }
                Err(error) => {
                    if let Some(trace) = &self.voice_trace {
                        trace.finish(utterance_id, "ai_error");
                    }
                    let message = format!("AI 翻译失败：{error:#}");
                    if self.voice_pending.is_empty()
                        && self.voice_active_utterance_id == utterance_id
                    {
                        self.finish_voice_error_for(utterance_id, message);
                    } else {
                        self.system_tray.set_status(&message);
                    }
                }
            }
        }
        if !recognized_this_frame
            && self.voice_translation.is_none()
            && let Some((utterance_id, text)) = self.voice_pending.pop_front()
        {
            self.finish_voice_recognition(utterance_id, text, context);
        }
        if let Some(overlay) = &self.voice_overlay {
            let drawn_id = overlay.drawn_original_id();
            if drawn_id > self.voice_last_logged_drawn_original_id {
                self.voice_last_logged_drawn_original_id = drawn_id;
                if let Some(trace) = &self.voice_trace {
                    trace.record(drawn_id, "overlay_original_drawn", None, "ok");
                }
            }
        }
        if self
            .voice_overlay
            .as_ref()
            .is_some_and(VoiceOverlay::expired)
        {
            if let Some(trace) = &self.voice_trace {
                trace.record(0, "overlay_hidden", None, "ok");
            }
            self.voice_overlay = None;
        }
    }

    fn finish_voice_recognition(
        &mut self,
        utterance_id: u64,
        text: String,
        context: &egui::Context,
    ) {
        let lookup_started = Instant::now();
        if let Some(trace) = &self.voice_trace {
            trace.record(utterance_id, "lookup_started", None, "ok");
        }
        let query = text.trim();
        if voice_word_cache::is_obviously_bad(query) {
            if let Some(trace) = &self.voice_trace {
                trace.finish(utterance_id, "empty_transcript");
            }
            self.finish_voice_error_for(utterance_id, "没有识别到清晰的语音，请重试".to_string());
            return;
        }
        self.voice_context
            .push_back((utterance_id, query.to_owned()));
        if self.voice_context.len() > 20 {
            self.voice_context.pop_front();
        }
        if utterance_id == self.voice_active_utterance_id
            && let Some(overlay) = &mut self.voice_overlay
        {
            overlay.original = query.to_string();
            overlay.result.clear();
            overlay.status = "正在查询本地词库…".to_string();
        }
        if same_language_no_translation(
            &self.config.source_language,
            &self.config.translation_language,
        ) {
            if let Some(trace) = &self.voice_trace {
                trace.record(
                    utterance_id,
                    "translation_skipped_same_language",
                    Some(lookup_started.elapsed()),
                    "ok",
                );
            }
            self.finish_voice_original(utterance_id, query);
            return;
        }
        if self.config.source_language == "auto"
            && let Some(detected) = translation_language::detect_source_language(query)
            && detected != self.config.translation_language
            && detected != self.active_source_language
        {
            match load_user_words(&self.config, detected) {
                Ok(words) => {
                    self.words = words;
                    self.active_source_language = detected.to_string();
                    self.word_load_error = None;
                    self.last_query.clear();
                    self.matches.clear();
                }
                Err(error) => {
                    if let Some(trace) = &self.voice_trace {
                        trace.record(
                            utterance_id,
                            "lookup_completed",
                            Some(lookup_started.elapsed()),
                            "error",
                        );
                        trace.finish(utterance_id, "dictionary_load_error");
                    }
                    self.finish_voice_error_for(
                        utterance_id,
                        format!("读取语音对应词库失败：{error:#}"),
                    );
                    return;
                }
            }
        }
        if let Some(error) = &self.word_load_error {
            if let Some(trace) = &self.voice_trace {
                trace.record(
                    utterance_id,
                    "lookup_completed",
                    Some(lookup_started.elapsed()),
                    "error",
                );
                trace.finish(utterance_id, "dictionary_load_error");
            }
            self.finish_voice_error_for(utterance_id, error.clone());
            return;
        }
        let (hits, search_query) = search_voice_words(&self.words, query);
        if let Some(trace) = &self.voice_trace {
            trace.record(
                utterance_id,
                "lookup_completed",
                Some(lookup_started.elapsed()),
                if hits.is_empty() { "miss" } else { "hit" },
            );
        }
        if let Some(first) = hits.first() {
            if hits.len() > 1 && !self.config.voice_auto_copy_first {
                if let Some(trace) = &self.voice_trace {
                    trace.finish(utterance_id, "manual_selection_required");
                }
                if let Some(mut session) = self.voice_session.take() {
                    session.stop();
                }
                self.voice_pending.clear();
                self.voice_overlay = None;
                self.open_popup(context);
                self.input = search_query.to_string();
                self.refresh_matches(context);
                self.system_tray
                    .set_status("语音找到多条词库结果，请在查询框选择");
                return;
            }
            let Some(entry) = self.words.get(first.entry_id) else {
                if let Some(trace) = &self.voice_trace {
                    trace.finish(utterance_id, "dictionary_entry_missing");
                }
                return;
            };
            let output = voice_lookup_output(entry, search_query).to_string();
            if utterance_id == self.voice_active_utterance_id
                && let Some(overlay) = &mut self.voice_overlay
            {
                overlay.result = output.clone();
            }
            let input_started = Instant::now();
            let typed = VoiceInputTarget::capture().map_or(Ok(None), |target| {
                target.type_if_active(
                    &output,
                    self.config.voice_aion2_manual_paste,
                    self.config.voice_hold_to_talk,
                )
            });
            if let Some(trace) = &self.voice_trace {
                trace.record(
                    utterance_id,
                    "text_input",
                    Some(input_started.elapsed()),
                    match &typed {
                        Ok(Some(VoiceInputMethod::Paste)) => "paste_sent",
                        Ok(Some(VoiceInputMethod::Unicode)) => "unicode_sent",
                        Ok(Some(VoiceInputMethod::ManualPaste)) => "manual_paste_ready",
                        #[cfg(feature = "tsf-notepad-prototype")]
                        Ok(Some(VoiceInputMethod::TextService)) => "text_service_queued",
                        Ok(None) => "skipped",
                        Err(error) => input_error_trace_code(error),
                    },
                );
                trace.finish(
                    utterance_id,
                    match &typed {
                        Ok(Some(_)) => "dictionary_input_sent",
                        Ok(None) => "input_skipped",
                        Err(_) => "input_error",
                    },
                );
            }
            match typed {
                Ok(Some(method)) => {
                    if hits.len() > 1 && method == VoiceInputMethod::Paste {
                        self.finish_voice_status_for(
                            utterance_id,
                            &format!("已发送 Ctrl+V（首条词库结果，共 {} 条匹配）", hits.len()),
                        );
                    } else {
                        self.finish_voice_status_for(
                            utterance_id,
                            voice_input_status(false, method),
                        );
                    }
                }
                Ok(None) => {
                    self.finish_voice_status_for(
                        utterance_id,
                        "当前窗口不可用或焦点已变化，未输入词库结果",
                    );
                }
                Err(reason) => {
                    self.finish_voice_status_for(
                        utterance_id,
                        &format!("词库结果输入失败：{reason:#}"),
                    );
                }
            }
            return;
        }
        if !self.ai_available() {
            if let Some(trace) = &self.voice_trace {
                trace.finish(utterance_id, "ai_unavailable");
            }
            self.finish_voice_error_for(
                utterance_id,
                "词库无匹配；请在设置中启用 DeepSeek 翻译并选择目标语言".to_string(),
            );
            return;
        }
        let credentials_started = Instant::now();
        let key_result = credentials::load_key();
        if let Some(trace) = &self.voice_trace {
            trace.record(
                utterance_id,
                "credential_load",
                Some(credentials_started.elapsed()),
                if key_result.is_ok() { "ok" } else { "error" },
            );
        }
        let key = match key_result {
            Ok(Some(key)) if !key.trim().is_empty() => key,
            Ok(_) => {
                if let Some(trace) = &self.voice_trace {
                    trace.finish(utterance_id, "api_key_missing");
                }
                self.finish_voice_error_for(
                    utterance_id,
                    "词库无匹配；请先设置 DeepSeek API Key".to_string(),
                );
                return;
            }
            Err(error) => {
                if let Some(trace) = &self.voice_trace {
                    trace.finish(utterance_id, "credential_error");
                }
                self.finish_voice_error_for(
                    utterance_id,
                    format!("读取 DeepSeek API Key 失败：{error}"),
                );
                return;
            }
        };
        let source = query.to_string();
        let source_language = self.active_source_language.clone();
        let target_language = self.config.translation_language.clone();
        let polite_mode = self.config.ai_polite_mode;
        let reverse_language = if self.config.source_language == "auto" {
            "cn".to_string()
        } else {
            self.config.source_language.clone()
        };
        let (sender, receiver) = mpsc::channel();
        let trace = self.voice_trace.clone();
        if let Some(trace) = &trace {
            trace.record(utterance_id, "ai_queued", None, "ok");
        }
        std::thread::spawn(move || {
            let ai_started = Instant::now();
            if let Some(trace) = &trace {
                trace.record(utterance_id, "ai_request_started", None, "ok");
            }
            let result = ai::translate(
                source,
                source_language,
                target_language,
                reverse_language,
                key,
                polite_mode,
            );
            if let Some(trace) = &trace {
                trace.record(
                    utterance_id,
                    "ai_request_completed",
                    Some(ai_started.elapsed()),
                    if result.is_ok() { "ok" } else { "error" },
                );
            }
            let _ = sender.send(result);
        });
        self.voice_translation = Some((utterance_id, receiver));
        if utterance_id == self.voice_active_utterance_id
            && let Some(overlay) = &mut self.voice_overlay
        {
            overlay.status = "本地词库无匹配，正在 AI 翻译…".to_string();
            overlay.keep_visible();
        }
    }

    fn finish_voice_original(&mut self, utterance_id: u64, text: &str) {
        let started = Instant::now();
        let typed = VoiceInputTarget::capture().map_or(Ok(None), |target| {
            target.type_if_active(
                text,
                self.config.voice_aion2_manual_paste,
                self.config.voice_hold_to_talk,
            )
        });
        if let Some(trace) = &self.voice_trace {
            trace.record(
                utterance_id,
                "text_input",
                Some(started.elapsed()),
                match &typed {
                    Ok(Some(VoiceInputMethod::Paste)) => "paste_sent",
                    Ok(Some(VoiceInputMethod::Unicode)) => "unicode_sent",
                    Ok(Some(VoiceInputMethod::ManualPaste)) => "manual_paste_ready",
                    #[cfg(feature = "tsf-notepad-prototype")]
                    Ok(Some(VoiceInputMethod::TextService)) => "text_service_queued",
                    Ok(None) => "skipped",
                    Err(error) => input_error_trace_code(error),
                },
            );
            trace.finish(
                utterance_id,
                match &typed {
                    Ok(Some(_)) => "original_input_sent",
                    Ok(None) => "input_skipped",
                    Err(_) => "input_error",
                },
            );
        }
        match typed {
            Ok(Some(method)) => {
                self.finish_voice_status_for(utterance_id, voice_original_status(method));
            }
            Ok(None) => self.finish_voice_status_for(
                utterance_id,
                "当前窗口不可用或焦点已变化，未输入识别原文",
            ),
            Err(error) => {
                self.finish_voice_status_for(utterance_id, &format!("识别原文输入失败：{error:#}"))
            }
        }
    }

    fn finish_voice_status(&mut self, status: &str) {
        self.system_tray.set_status(status);
        if let Some(overlay) = &mut self.voice_overlay {
            overlay.status = status.to_string();
            overlay.finish_after(VOICE_OVERLAY_HOLD);
        }
        if let Some(trace) = &self.voice_trace {
            trace.record(0, "overlay_hide_scheduled", Some(VOICE_OVERLAY_HOLD), "ok");
        }
    }

    fn finish_voice_status_for(&mut self, utterance_id: u64, status: &str) {
        if utterance_id == self.voice_active_utterance_id {
            self.finish_voice_status(status);
        } else {
            self.system_tray.set_status(status);
        }
    }

    fn finish_voice_error(&mut self, error: String) {
        self.system_tray.set_status(&error);
        if let Some(overlay) = &mut self.voice_overlay {
            overlay.status = error;
            overlay.finish_after(VOICE_OVERLAY_HOLD);
        }
        if let Some(trace) = &self.voice_trace {
            trace.record(
                0,
                "overlay_hide_scheduled",
                Some(VOICE_OVERLAY_HOLD),
                "error",
            );
        }
    }

    fn finish_voice_error_for(&mut self, utterance_id: u64, error: String) {
        if utterance_id == self.voice_active_utterance_id {
            self.finish_voice_error(error);
        } else {
            self.system_tray.set_status(&error);
        }
    }

    fn apply_settings(&mut self, config: UiConfig, key_update: &AiKeyUpdate) -> Result<()> {
        let config = config.normalized();
        let new_hotkeys = parse_hotkeys(&config)?;
        if self.mouse_hotkeys.is_none()
            && [
                new_hotkeys.popup,
                new_hotkeys.settings,
                new_hotkeys.clear_composed,
                new_hotkeys.ocr,
                new_hotkeys.voice,
            ]
            .into_iter()
            .any(|shortcut| shortcut.mouse().is_some())
        {
            bail!("鼠标快捷键监听不可用，无法启用鼠标按键绑定");
        }
        if new_hotkeys.uses_extra_mouse_button() && self.extra_mouse_buttons.is_none() {
            bail!(
                "当前鼠标扩展键监听不可用；第 6～8 键需由驱动提供 DirectInput 按键，或在鼠标驱动中映射成独立键盘快捷键"
            );
        }
        let new_source = if config.source_language == "auto" {
            self.active_source_language.clone()
        } else {
            config.source_language.clone()
        };
        let new_words = load_user_words(&config, &new_source)?;
        match key_update {
            AiKeyUpdate::Unchanged => {}
            AiKeyUpdate::Set(key) => {
                credentials::save_key(key).context("保存 DeepSeek API Key 失败")?
            }
            AiKeyUpdate::Delete => {
                credentials::delete_key().context("删除 DeepSeek API Key 失败")?
            }
        }
        let old_keyboard_keys = self.hotkeys.global_keyboard_keys();
        let new_keyboard_keys = new_hotkeys.global_keyboard_keys();
        let keyboard_changed = old_keyboard_keys != new_keyboard_keys;

        if keyboard_changed {
            self.hotkey_manager
                .unregister_all(&old_keyboard_keys)
                .context("无法取消注册原快捷键")?;
            if let Err(error) = register_global_keyboard_hotkeys(&self.hotkey_manager, new_hotkeys)
            {
                let _ = self.hotkey_manager.register_all(&old_keyboard_keys);
                return Err(error);
            }
        }

        if let Err(error) = save_ui_config(&config) {
            if keyboard_changed {
                let _ = self.hotkey_manager.unregister_all(&new_keyboard_keys);
                let _ = self.hotkey_manager.register_all(&old_keyboard_keys);
            }
            return Err(error);
        }

        self.hotkeys = new_hotkeys;
        self.mouse_bindings_active_at = Instant::now();
        self.config = config;
        self.words = new_words;
        self.active_source_language = new_source;
        self.word_load_error = None;
        self.matches.clear();
        self.ai_state = AiState::Idle;
        self.last_ai_candidate = None;
        self.last_query.clear();
        Ok(())
    }

    fn sync_shortcut_recording(&mut self) {
        let should_suspend = self.settings_open && self.settings_panel.is_recording_shortcut();
        let hotkeys = self.hotkeys.global_keyboard_keys();

        if should_suspend && !self.hotkeys_suspended {
            match self.hotkey_manager.unregister_all(&hotkeys) {
                Ok(()) => self.hotkeys_suspended = true,
                Err(error) => self
                    .settings_panel
                    .set_result(Err(format!("开始录制快捷键失败：{error}"))),
            }
        } else if !should_suspend && self.hotkeys_suspended {
            match self.hotkey_manager.register_all(&hotkeys) {
                Ok(()) => self.hotkeys_suspended = false,
                Err(error) => self
                    .settings_panel
                    .set_result(Err(format!("恢复原快捷键失败：{error}"))),
            }
        }
    }

    fn refresh_matches(&mut self, context: &egui::Context) {
        self.ai_state = AiState::Idle;
        let query = self.input.trim().to_owned();
        self.selected = 0;
        self.result_page = 0;
        self.no_match_delete_armed = false;
        self.message = None;

        if query.is_empty() {
            self.matches.clear();
            self.last_query.clear();
            self.resize_window(context);
            return;
        }

        if self.config.source_language == "auto"
            && let Some(detected) = translation_language::detect_source_language(&query)
        {
            // 目标语文字也能用来反查当前词库中的 key，不切到“目标语→目标语”。
            if detected != self.config.translation_language
                && detected != self.active_source_language
            {
                match load_user_words(&self.config, detected) {
                    Ok(words) => {
                        self.words = words;
                        self.active_source_language = detected.to_string();
                        self.word_load_error = None;
                        self.last_query.clear();
                        self.matches.clear();
                    }
                    Err(error) => {
                        self.word_load_error = Some(format!("读取词条失败：{error:#}"));
                    }
                }
            }
        }

        if let Some(error) = &self.word_load_error {
            self.matches.clear();
            self.last_query.clear();
            self.message = Some(error.clone());
        } else {
            self.matches = self.words.search(&query, &self.last_query, &self.matches);
            self.last_query = query;
        }
        self.resize_window(context);
    }

    fn reload_words(&mut self, context: &egui::Context) -> Result<usize> {
        match load_user_words(&self.config, &self.active_source_language) {
            Ok(words) => {
                let count = words.len();
                self.words = words;
                self.word_load_error = None;
                self.matches.clear();
                self.last_query.clear();
                if self.is_open && !self.settings_open && !self.input.trim().is_empty() {
                    self.refresh_matches(context);
                }
                Ok(count)
            }
            Err(error) => {
                let message = format!("读取词条失败：{error:#}");
                self.word_load_error = Some(message.clone());
                Err(anyhow::anyhow!(message))
            }
        }
    }

    fn resize_window(&mut self, context: &egui::Context) {
        let has_composed = !self.continuous_input.is_empty();
        let has_query_result = !self.input.trim().is_empty()
            || self.message.is_some()
            || self.last_ai_candidate.is_some();
        let has_result_area = has_composed || has_query_result;
        let mut width = self.config.input.width;
        let mut height = self.config.input.height;

        if has_result_area {
            let visible_range = self.visible_match_range();
            let mut estimated_width = self.matches[visible_range.clone()]
                .iter()
                .filter_map(|hit| self.words.get(hit.entry_id))
                .map(|entry| estimate_text_width(entry, self.config.results.font_size))
                .fold(self.config.results.min_width, f32::max)
                .max(if self.message.is_some() { 280.0 } else { 0.0 });
            if let AiState::Ready(candidate) = &self.ai_state {
                estimated_width = estimated_width.max(estimate_plain_text_width(
                    &format!("{}（{}）", candidate.translated, candidate.source),
                    self.config.results.font_size,
                ));
            }
            if self.last_ai_candidate.is_some() {
                estimated_width = estimated_width.max(330.0);
            }
            if has_composed {
                estimated_width = estimated_width.max(estimate_plain_text_width(
                    self.continuous_input.display_text(),
                    self.config.results.font_size,
                ));
            }
            width = width.max(
                estimated_width.clamp(self.config.results.min_width, self.config.results.max_width),
            );

            let content_width = (width - 14.0).max(40.0);
            let mut result_height = 12.0;
            if has_composed {
                let lines = (estimate_plain_text_width(
                    self.continuous_input.display_text(),
                    self.config.results.font_size,
                ) / content_width)
                    .ceil()
                    .max(1.0);
                result_height += (lines * self.config.results.font_size * 1.45 + 8.0)
                    .max(self.config.results.min_row_height);
            }
            if has_query_result {
                if has_composed {
                    result_height += 4.0;
                }
                if self.matches.len() > self.config.results.max_visible_results {
                    result_height += self.config.results.font_size * 1.4 + 3.0;
                }
                result_height += if self.matches.is_empty() {
                    if self.message.is_some()
                        || (!self.input.trim().is_empty()
                            && !matches!(self.ai_state, AiState::Ready(_)))
                    {
                        self.config.results.min_row_height
                    } else {
                        0.0
                    }
                } else {
                    self.matches[visible_range]
                        .iter()
                        .filter_map(|hit| self.words.get(hit.entry_id))
                        .map(|entry| {
                            let lines = (estimate_text_width(entry, self.config.results.font_size)
                                / content_width)
                                .ceil()
                                .max(1.0);
                            (lines * self.config.results.font_size * 1.45 + 8.0)
                                .max(self.config.results.min_row_height)
                        })
                        .sum::<f32>()
                };
                if let AiState::Ready(candidate) = &self.ai_state {
                    let label =
                        format!("AI 译文：{}（{}）", candidate.translated, candidate.source);
                    let lines = (estimate_plain_text_width(&label, self.config.results.font_size)
                        / content_width)
                        .ceil()
                        .max(1.0);
                    result_height += (lines * self.config.results.font_size * 1.45 + 8.0)
                        .max(self.config.results.min_row_height);
                }
                if !self.matches.is_empty() && self.message.is_some() {
                    result_height += self.config.results.min_row_height;
                }
                if !self.input.trim().is_empty()
                    && self.ai_available()
                    && !matches!(self.ai_state, AiState::Loading(_) | AiState::Ready(_))
                {
                    result_height += self.config.results.min_row_height + 8.0;
                }
            }
            if self.last_ai_candidate.is_some() {
                result_height += self.config.results.min_row_height + 8.0;
            }
            let result_height = result_height.min(self.config.results.max_height);
            height += self.config.gap + result_height;
        }

        let new_size = egui::vec2(width.ceil(), height.ceil());
        if (new_size - self.viewport_size).length_sq() > 1.0 {
            self.viewport_size = new_size;
            context.send_viewport_cmd(egui::ViewportCommand::InnerSize(new_size));
        } else {
            context.send_viewport_cmd(egui::ViewportCommand::InnerSize(self.viewport_size));
        }
        if !self.manually_moved {
            self.position_window(context);
        }
    }

    fn position_window(&self, context: &egui::Context) {
        let Some(anchor) = self.popup_anchor else {
            return;
        };
        context.send_viewport_cmd(egui::ViewportCommand::OuterPosition(popup_position(
            anchor,
            self.viewport_size,
        )));
    }

    fn visible_match_range(&self) -> std::ops::Range<usize> {
        let page_size = self.config.results.max_visible_results.max(1);
        let start = (self.result_page * page_size).min(self.matches.len());
        start..(start + page_size).min(self.matches.len())
    }

    fn commit_current_with_space(&mut self, context: &egui::Context) {
        if let AiState::Ready(candidate) = &self.ai_state {
            self.accept_ai_candidate(candidate.clone(), context, true);
        } else if !self.matches.is_empty() {
            self.accept_match(self.selected, context, true);
        } else {
            self.message = Some("没有匹配结果，当前文字未加入组合".to_string());
            self.resize_window(context);
        }
    }

    fn handle_keyboard(&mut self, context: &egui::Context) {
        if context.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.hide(context);
            return;
        }
        let clear_composed = shortcut_pressed(context, &self.config.hotkeys.clear_composed);
        let undo_composed = context.input(|input| {
            input.key_pressed(egui::Key::Backspace)
                && !input.modifiers.ctrl
                && !input.modifiers.alt
                && !input.modifiers.shift
                && !input.modifiers.command
        });
        if (clear_composed && self.change_composed(context, true))
            || (undo_composed && self.change_composed(context, false))
        {
            return;
        }
        if context.input(|input| input.key_pressed(egui::Key::ArrowLeft)) && self.result_page > 0 {
            self.result_page -= 1;
            self.selected = self.visible_match_range().start;
            self.resize_window(context);
        }
        if context.input(|input| input.key_pressed(egui::Key::ArrowRight)) {
            let page_size = self.config.results.max_visible_results.max(1);
            if (self.result_page + 1) * page_size < self.matches.len() {
                self.result_page += 1;
                self.selected = self.visible_match_range().start;
                self.resize_window(context);
            }
        }
        if context.input(|input| input.key_pressed(egui::Key::ArrowUp)) {
            if self.matches.is_empty() {
                return;
            }
            self.selected = self
                .selected
                .saturating_sub(1)
                .max(self.visible_match_range().start);
        }
        if context.input(|input| input.key_pressed(egui::Key::ArrowDown)) {
            if self.matches.is_empty() {
                return;
            }
            self.selected = (self.selected + 1).min(self.visible_match_range().end - 1);
        }
        if context.input(|input| input.modifiers.ctrl && input.key_pressed(egui::Key::Enter)) {
            self.request_ai_translation(context);
            return;
        }
        if context.input(|input| input.key_pressed(egui::Key::Enter)) {
            if let AiState::Ready(candidate) = &self.ai_state {
                self.accept_ai_candidate(candidate.clone(), context, false);
            } else if self.matches.is_empty() {
                self.handle_unmatched_enter(context);
            } else {
                self.accept_match(self.selected, context, false);
            }
            if self.is_open {
                self.request_input_focus = true;
            }
        }
    }

    fn change_composed(&mut self, context: &egui::Context, clear_all: bool) -> bool {
        if !self.input.is_empty() || self.continuous_input.is_empty() {
            return false;
        }
        if clear_all {
            self.continuous_input.clear();
        } else {
            self.continuous_input.undo();
        }
        if self.copied_since_focus {
            context.copy_text(self.continuous_input.clipboard_text().to_owned());
        }
        if self.continuous_input.is_empty() {
            self.copied_since_focus = false;
            self.left_after_copy = false;
        }
        self.message = None;
        self.request_input_focus = true;
        self.resize_window(context);
        true
    }

    fn handle_unmatched_enter(&mut self, context: &egui::Context) {
        if matches!(self.ai_state, AiState::Loading(_)) {
            return;
        }
        if self.input.trim().is_empty() {
            if !self.continuous_input.is_empty() {
                context.copy_text(self.continuous_input.clipboard_text().to_owned());
                self.copied_since_focus = true;
                self.left_after_copy = false;
            }
            if !self.config.continuous_input {
                self.hide(context);
            }
            return;
        }
        if self.message.as_deref().is_some_and(|message| {
            message.starts_with("读取词条失败：") || message.starts_with("UI 配置错误：")
        }) {
            return;
        }

        if self.no_match_delete_armed {
            self.input.clear();
            self.matches.clear();
            self.last_query.clear();
            self.selected = 0;
            self.no_match_delete_armed = false;
            self.message = None;
            self.request_input_focus = true;
        } else {
            self.no_match_delete_armed = true;
            self.message = Some("没有匹配结果，再按一次 Enter 清空输入".to_string());
        }
        self.resize_window(context);
    }

    fn accept_match(&mut self, index: usize, context: &egui::Context, continue_input: bool) {
        let Some(entry) = self
            .matches
            .get(index)
            .and_then(|hit| self.words.get(hit.entry_id))
        else {
            return;
        };
        let key = entry.key.clone();
        let value = entry.value.clone();
        self.accept_text(&key, &value, context, continue_input);
    }

    fn accept_ai_candidate(
        &mut self,
        candidate: AiCandidate,
        context: &egui::Context,
        continue_input: bool,
    ) {
        self.accept_text(
            &candidate.translated,
            &candidate.source,
            context,
            continue_input,
        );
        self.last_ai_candidate = Some(candidate);
        if self.config.ai_auto_save {
            self.save_last_ai_candidate(context);
        } else {
            self.resize_window(context);
        }
    }

    fn accept_text(
        &mut self,
        key: &str,
        value: &str,
        context: &egui::Context,
        continue_input: bool,
    ) {
        let composed = self.continuous_input.append(key, value).to_owned();
        if !continue_input {
            context.copy_text(composed);
            self.copied_since_focus = true;
            self.left_after_copy = false;
        }
        self.input.clear();
        self.matches.clear();
        self.ai_state = AiState::Idle;
        self.last_query.clear();
        self.selected = 0;
        self.result_page = 0;
        self.no_match_delete_armed = false;
        self.message = None;
        self.request_input_focus = true;
        self.resize_window(context);
        if !continue_input && !self.config.continuous_input {
            self.hide(context);
        }
    }

    fn ai_available(&self) -> bool {
        self.config.ai_translation
            && !self.config.translation_language.is_empty()
            && self.word_load_error.is_none()
    }

    fn request_ai_translation(&mut self, context: &egui::Context) {
        if !self.ai_available()
            || self.input.trim().is_empty()
            || matches!(self.ai_state, AiState::Loading(_))
        {
            return;
        }
        let key = match credentials::load_key() {
            Ok(Some(key)) if !key.trim().is_empty() => key,
            Ok(_) => {
                self.message = Some("请先在设置中填写 DeepSeek API Key".to_string());
                self.resize_window(context);
                return;
            }
            Err(error) => {
                self.message = Some(format!("读取 DeepSeek API Key 失败：{error}"));
                self.resize_window(context);
                return;
            }
        };
        let source = self.input.trim().to_owned();
        let source_language = self.active_source_language.clone();
        let language = self.config.translation_language.clone();
        let polite_mode = self.config.ai_polite_mode;
        let reverse_language = if self.config.source_language == "auto" {
            "cn".to_string()
        } else {
            self.config.source_language.clone()
        };
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(ai::translate(
                source,
                source_language,
                language,
                reverse_language,
                key,
                polite_mode,
            ));
        });
        self.ai_state = AiState::Loading(receiver);
        self.message = Some("DeepSeek 翻译中…".to_string());
        self.no_match_delete_armed = false;
        self.resize_window(context);
    }

    fn poll_ai_translation(&mut self, context: &egui::Context) {
        let result = match &self.ai_state {
            AiState::Loading(receiver) => match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(TryRecvError::Disconnected) => Some(Err(anyhow::anyhow!("翻译任务中断"))),
                Err(TryRecvError::Empty) => None,
            },
            _ => None,
        };
        if let Some(result) = result {
            match result {
                Ok(candidate)
                    if candidate.source == self.input.trim()
                        && candidate.language_code == self.config.translation_language
                        && candidate.source_language_code
                            == if candidate.direction == TranslationDirection::Reverse {
                                if self.config.source_language == "auto" {
                                    "cn"
                                } else {
                                    &self.config.source_language
                                }
                            } else {
                                &self.active_source_language
                            } =>
                {
                    self.ai_state = AiState::Ready(candidate);
                    self.message = None;
                }
                Ok(_) => self.ai_state = AiState::Idle,
                Err(error) => {
                    let message = format!("AI 翻译失败：{error:#}");
                    self.ai_state = AiState::Error;
                    self.message = Some(message);
                }
            }
            self.resize_window(context);
        }
    }

    fn save_last_ai_candidate(&mut self, context: &egui::Context) {
        if let Some(candidate) = &self.last_ai_candidate {
            let saved_to_active_library =
                candidate.source_language_code == self.active_source_language;
            match ai_word_save::save_candidate(candidate) {
                Ok(()) => {
                    self.last_ai_candidate = None;
                    self.message = Some(if saved_to_active_library {
                        match self.reload_words(context) {
                            Ok(_) => "AI 译文已保存到当前词库".to_string(),
                            Err(error) => format!("词库已保存，但重新加载失败：{error:#}"),
                        }
                    } else {
                        "AI 译文已保存到对应语言词库".to_string()
                    });
                }
                Err(error) => self.message = Some(format!("保存 AI 译文失败：{error:#}")),
            }
            self.resize_window(context);
        }
    }

    fn handle_popup_focus_change(&mut self, context: &egui::Context) {
        let (lost_focus, gained_focus) = context.input(|input| {
            let mut lost = false;
            let mut gained = false;
            for event in &input.events {
                match event {
                    egui::Event::WindowFocused(false) => lost = true,
                    egui::Event::WindowFocused(true) => gained = true,
                    _ => {}
                }
            }
            (lost, gained)
        });

        if lost_focus && self.copied_since_focus {
            self.left_after_copy = true;
        }
        if gained_focus {
            if self.config.clear_on_focus_return && self.copied_since_focus && self.left_after_copy
            {
                self.input.clear();
                self.matches.clear();
                self.last_query.clear();
                self.continuous_input.clear();
                self.selected = 0;
                self.result_page = 0;
                self.no_match_delete_armed = false;
                self.message = None;
                self.copied_since_focus = false;
                self.left_after_copy = false;
                self.resize_window(context);
            }
            self.request_input_focus = true;
        }
    }

    fn draw_result(&mut self, ui: &mut egui::Ui, index: usize) -> bool {
        let Some(entry) = self
            .matches
            .get(index)
            .and_then(|hit| self.words.get(hit.entry_id))
        else {
            return false;
        };
        let is_selected = self.selected == index && !matches!(self.ai_state, AiState::Ready(_));
        let fill = if is_selected {
            self.config.selected_color()
        } else {
            egui::Color32::TRANSPARENT
        };
        let text_color = self.config.text_color();
        let key_color = if is_selected {
            self.config.selected_text_color()
        } else {
            text_color
        };

        let response = egui::Frame::new()
            .fill(fill)
            .corner_radius(self.config.results.corner_radius)
            .inner_margin(egui::Margin::symmetric(7, 4))
            .show(ui, |ui| {
                ui.set_min_height(self.config.results.min_row_height - 8.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(highlighted_match_text(
                        &entry.key,
                        self.input.trim(),
                        self.config.results.font_size,
                        key_color,
                        self.config.selected_text_color(),
                    ));
                    ui.add_space(4.0);
                    ui.label(highlighted_match_text(
                        &entry.value,
                        self.input.trim(),
                        self.config.results.font_size,
                        text_color.gamma_multiply(0.78),
                        self.config.selected_text_color(),
                    ));
                });
            })
            .response
            .interact(egui::Sense::click());

        let clicked = response.clicked();
        if clicked {
            self.selected = index;
            self.accept_match(index, ui.ctx(), false);
        }
        if is_selected {
            let indicator = egui::Rect::from_min_max(
                egui::pos2(response.rect.left() + 2.0, response.rect.top() + 5.0),
                egui::pos2(response.rect.left() + 5.0, response.rect.bottom() - 5.0),
            );
            ui.painter()
                .rect_filled(indicator, 2.0, self.config.selected_text_color());
            response.scroll_to_me(Some(egui::Align::Center));
        }
        clicked
    }
}

impl eframe::App for MatchApp {
    fn logic(&mut self, context: &egui::Context, frame: &mut eframe::Frame) {
        while let Some(action) = self.system_tray.poll_action() {
            match action {
                TrayAction::OpenQuery => {
                    self.open_popup(context);
                    reveal_native_window(frame);
                }
                TrayAction::OpenSettings => {
                    self.open_settings(context);
                    reveal_native_window(frame);
                }
                TrayAction::ReloadWords => match self.reload_words(context) {
                    Ok(count) => self
                        .system_tray
                        .set_status(&format!("已重新加载 {count} 个词条")),
                    Err(error) => self
                        .system_tray
                        .set_status(&format!("词库加载失败：{error:#}")),
                },
                TrayAction::Exit => {
                    self.exit_requested = true;
                    context.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
            }
        }
        if self.settings_panel.take_word_reload_requested() {
            match self.reload_words(context) {
                Ok(count) => self
                    .system_tray
                    .set_status(&format!("已载入 {count} 个词条")),
                Err(error) => self
                    .system_tray
                    .set_status(&format!("词库加载失败：{error:#}")),
            }
        }
        self.sync_shortcut_recording();
        self.poll_ai_translation(context);
        self.poll_ocr(context);
        self.poll_voice(context);
        let recording_shortcut = self.settings_open && self.settings_panel.is_recording_shortcut();
        let extra_needed = recording_shortcut || self.hotkeys.uses_extra_mouse_button();
        if extra_needed && !self.extra_mouse_attempted {
            if let Some(window) = frame.winit_window() {
                use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
                if let Ok(handle) = window.window_handle() {
                    self.extra_mouse_attempted = true;
                    if let RawWindowHandle::Win32(handle) = handle.as_raw() {
                        match ExtraMouseButtons::new(handle.hwnd.get()) {
                            Ok(listener) => self.extra_mouse_buttons = Some(listener),
                            Err(error) if self.hotkeys.uses_extra_mouse_button() => self
                                .system_tray
                                .set_status(&format!("鼠标扩展键监听不可用：{error:#}")),
                            Err(_) => {}
                        }
                    }
                }
            }
        }
        // 设置窗口拥有焦点时，部分环境不会把组合键送到全局热键通道。
        // 同时读取 egui 的本地按键事件，并与全局事件合并，避免重复打开。
        let mut open_query = self.settings_open
            && !recording_shortcut
            && shortcut_pressed(context, &self.config.hotkeys.popup);
        let mut open_settings = false;
        let mut open_ocr = false;
        let mut toggle_voice = self.settings_open
            && !recording_shortcut
            && shortcut_pressed(context, &self.config.hotkeys.voice);
        let mut release_voice =
            self.settings_open && shortcut_released(context, &self.config.hotkeys.voice);
        for event in GlobalHotKeyEvent::receiver().try_iter() {
            if Some(event.id) == self.hotkeys.voice.keyboard_id()
                && event.state == HotKeyState::Released
            {
                release_voice = true;
                continue;
            }
            if event.state != HotKeyState::Pressed || recording_shortcut {
                continue;
            }
            if Some(event.id) == self.hotkeys.popup.keyboard_id() {
                open_query = true;
            } else if Some(event.id) == self.hotkeys.settings.keyboard_id() {
                open_settings = true;
            } else if Some(event.id) == self.hotkeys.ocr.keyboard_id() {
                open_ocr = true;
            } else if Some(event.id) == self.hotkeys.voice.keyboard_id() {
                toggle_voice = true;
            }
        }
        let mut mouse_events = Vec::new();
        if let Some(extra) = &mut self.extra_mouse_buttons {
            if extra_needed {
                mouse_events.extend(extra.poll());
            } else {
                extra.reset();
            }
        }
        while let Some(event) = self
            .mouse_hotkeys
            .as_ref()
            .and_then(MouseHotkeyListener::try_recv)
        {
            mouse_events.push(event);
        }
        for event in mouse_events {
            if recording_shortcut {
                self.settings_panel.capture_mouse_shortcut(event);
                continue;
            }
            if event.occurred_at < self.mouse_bindings_active_at {
                continue;
            }
            if self
                .hotkeys
                .voice
                .mouse()
                .is_some_and(|shortcut| shortcut.button == event.button)
                && !event.pressed
            {
                release_voice = true;
                continue;
            }
            if !event.pressed {
                continue;
            }
            let matches = |binding: Shortcut| {
                binding
                    .mouse()
                    .is_some_and(|shortcut| shortcut.matches(event.button, event.modifiers))
            };
            if matches(self.hotkeys.popup) {
                open_query = true;
            } else if matches(self.hotkeys.settings) {
                open_settings = true;
            } else if matches(self.hotkeys.ocr) {
                open_ocr = true;
            } else if matches(self.hotkeys.voice) {
                toggle_voice = true;
            } else if matches(self.hotkeys.clear_composed) && self.is_open && !self.settings_open {
                self.change_composed(context, true);
            }
        }
        if open_query {
            self.open_popup(context);
            reveal_native_window(frame);
        } else if open_settings {
            self.open_settings(context);
            reveal_native_window(frame);
        } else if open_ocr {
            self.begin_ocr_capture(context);
        } else if toggle_voice {
            self.toggle_voice(context);
        }
        if release_voice {
            self.stop_held_voice();
        }
        if let Some(overlay) = &self.voice_overlay {
            overlay.show(context);
        }

        let repaint_interval = if recording_shortcut || self.hotkeys.uses_extra_mouse_button() {
            Duration::from_millis(8)
        } else if self.settings_panel.has_open_dialog() {
            Duration::from_millis(16)
        } else {
            Duration::from_millis(40)
        };
        context.request_repaint_after(repaint_interval);
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        if self.exit_requested {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if !self.is_open {
            if let Some(window) = frame.winit_window() {
                window.set_visible(false);
            }
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Visible(false));
            return;
        }

        if matches!(self.ocr_state, OcrState::Selecting { .. }) {
            if let Some(window) = frame.winit_window() {
                if let OcrState::Selecting { selection, .. } = &self.ocr_state {
                    let monitor = selection.screen.monitor;
                    let target_position = winit::dpi::PhysicalPosition::new(monitor.x, monitor.y);
                    let target_size = winit::dpi::PhysicalSize::new(monitor.width, monitor.height);
                    if window.outer_position().ok() != Some(target_position) {
                        window.set_outer_position(target_position);
                    }
                    if window.inner_size() != target_size {
                        let _ = window.request_inner_size(target_size);
                    }
                }
            }
            let action = match &mut self.ocr_state {
                OcrState::Selecting { selection, preview } => selection.show(
                    ui,
                    preview
                        .display
                        .as_ref()
                        .map(|(text, error)| (text.as_str(), *error)),
                ),
                _ => None,
            };
            match action {
                Some(SelectionAction::Changed(region)) => {
                    if let OcrState::Selecting { preview, .. } = &mut self.ocr_state {
                        preview.schedule(region);
                    }
                }
                Some(SelectionAction::Cancel) => {
                    self.hide(ui.ctx());
                    self.system_tray.set_status("OCR 已取消");
                }
                Some(SelectionAction::Confirm(region)) => {
                    let OcrState::Selecting { selection, preview } = &self.ocr_state else {
                        unreachable!()
                    };
                    let recognized = (preview.region == Some(region))
                        .then(|| preview.recognized.clone())
                        .flatten();
                    if let Some(text) = recognized {
                        self.hide(ui.ctx());
                        self.copy_ocr_original(ui.ctx(), &text);
                    } else {
                        let screen: Arc<CapturedScreen> = Arc::clone(&selection.screen);
                        let source = self.config.source_language.clone();
                        let target = self.config.translation_language.clone();
                        let (sender, receiver) = mpsc::channel();
                        std::thread::spawn(move || {
                            let _ = sender
                                .send(ocr::recognize_selection(&screen, region, &source, &target));
                        });
                        self.hide(ui.ctx());
                        self.ocr_state = OcrState::Finalizing(receiver);
                    }
                }
                None => {}
            }
            return;
        }

        if self.settings_open {
            if ui.ctx().input(|input| input.viewport().close_requested()) {
                ui.ctx()
                    .send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.hide(ui.ctx());
                return;
            }
            if !self.settings_panel.is_recording_shortcut()
                && !self.settings_panel.has_open_dialog()
                && ui.ctx().input(|input| input.key_pressed(egui::Key::Escape))
            {
                self.hide(ui.ctx());
                return;
            }
            let settings_result = self.settings_panel.show(ui);
            if self.settings_panel.take_word_editor_review_requested()
                && self
                    .voice_save_requests
                    .send(VoiceSaveRequest::ReviewNow)
                    .is_err()
            {
                self.system_tray
                    .set_status("语音词库审核线程已停止，无法审核缓存");
            }
            if let Some((config, key_update)) = settings_result {
                let result = self
                    .apply_settings(config, &key_update)
                    .map_err(|error| format!("{error:#}"));
                if result.is_ok() {
                    self.settings_panel.mark_ai_key_saved(&key_update);
                }
                self.settings_panel.set_result(result);
            }
            return;
        }

        ui_theme::apply(ui);

        let input_rect = egui::Rect::from_min_size(ui.max_rect().min, self.config.input_size());
        ui.painter().rect_filled(
            input_rect,
            self.config.input.corner_radius,
            self.config.input_color(),
        );
        ui.painter().rect_stroke(
            input_rect,
            self.config.input.corner_radius,
            egui::Stroke::new(1.0, ui_theme::border().gamma_multiply(0.82)),
            egui::StrokeKind::Inside,
        );

        let close_width = self.config.input.height.min(26.0);
        let close_rect = egui::Rect::from_min_max(
            egui::pos2(input_rect.right() - close_width, input_rect.top()),
            input_rect.right_bottom(),
        );
        let close_response = ui.interact(
            close_rect,
            egui::Id::new("close_popup"),
            egui::Sense::click(),
        );
        if close_response.hovered() {
            ui.painter().circle_filled(
                close_rect.center(),
                close_width * 0.32,
                ui_theme::pale_red().gamma_multiply(0.78),
            );
        }
        ui.painter().text(
            close_rect.center(),
            egui::Align2::CENTER_CENTER,
            "×",
            egui::FontId::proportional(self.config.input.font_size + 1.0),
            self.config.text_color().gamma_multiply(0.70),
        );
        if close_response.clicked() {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Visible(false));
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }

        let edit_rect = egui::Rect::from_min_max(
            input_rect.min + egui::vec2(6.0, 1.0),
            egui::pos2(close_rect.left() - 2.0, input_rect.bottom() - 1.0),
        );
        self.handle_popup_focus_change(ui.ctx());
        let response = ui.put(
            edit_rect,
            egui::TextEdit::singleline(&mut self.input)
                .id(egui::Id::new(INPUT_ID))
                .hint_text("输入文字…")
                .font(egui::FontId::proportional(self.config.input.font_size))
                .text_color(self.config.text_color())
                .desired_width(edit_rect.width())
                .vertical_align(egui::Align::Center)
                .frame(egui::Frame::NONE)
                .event_filter(egui::EventFilter {
                    vertical_arrows: true,
                    horizontal_arrows: true,
                    ..Default::default()
                }),
        );

        if response.drag_started() {
            self.manually_moved = true;
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }
        if response.changed() {
            let continue_with_space = take_query_separator(&mut self.input);
            if !continue_with_space || self.input.trim() != self.last_query {
                self.refresh_matches(ui.ctx());
            }
            if continue_with_space {
                self.commit_current_with_space(ui.ctx());
            }
        }
        self.handle_keyboard(ui.ctx());
        if self.request_input_focus {
            response.request_focus();
            self.request_input_focus = false;
        }

        let has_composed = !self.continuous_input.is_empty();
        let has_query_result = !self.input.trim().is_empty()
            || self.message.is_some()
            || self.last_ai_candidate.is_some();
        if !has_composed && !has_query_result {
            return;
        }

        let result_top = self.config.input.height + self.config.gap;
        let result_rect = egui::Rect::from_min_max(
            ui.max_rect().min + egui::vec2(0.0, result_top),
            ui.max_rect().max,
        );
        ui.painter().rect_filled(
            result_rect,
            self.config.results.corner_radius,
            self.config.results_color(),
        );
        ui.painter().rect_stroke(
            result_rect,
            self.config.results.corner_radius,
            egui::Stroke::new(1.0, ui_theme::border().gamma_multiply(0.82)),
            egui::StrokeKind::Inside,
        );
        let result_drag = ui.interact(
            result_rect,
            egui::Id::new("result_area_drag"),
            egui::Sense::drag(),
        );
        if result_drag.drag_started() {
            self.manually_moved = true;
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
        }

        ui.scope_builder(
            egui::UiBuilder::new().max_rect(result_rect.shrink(6.0)),
            |ui| {
                if has_composed {
                    ui.label(
                        egui::RichText::new(format!(
                            "已组合：{}",
                            self.continuous_input.display_text()
                        ))
                        .size(self.config.results.font_size)
                        .strong()
                        .color(self.config.selected_text_color()),
                    );
                    if has_query_result {
                        ui.add_space(4.0);
                    }
                }

                if self.last_ai_candidate.is_some() {
                    if ui.button("将刚才的 AI 译文保存到词库").clicked() {
                        self.save_last_ai_candidate(ui.ctx());
                    }
                    ui.add_space(4.0);
                }

                let visible_range = self.visible_match_range();
                if self.matches.len() > self.config.results.max_visible_results {
                    ui.label(
                        egui::RichText::new(format!(
                            "结果 {}–{} / {}　← → 翻页",
                            visible_range.start + 1,
                            visible_range.end,
                            self.matches.len()
                        ))
                        .size((self.config.results.font_size - 1.0).max(8.0))
                        .color(self.config.text_color().gamma_multiply(0.68)),
                    );
                    ui.add_space(3.0);
                }

                if let AiState::Ready(candidate) = &self.ai_state {
                    let candidate = candidate.clone();
                    let label = egui::RichText::new(format!(
                        "AI 译文：{}（{}）",
                        candidate.translated, candidate.source
                    ))
                    .size(self.config.results.font_size)
                    .color(self.config.selected_text_color());
                    if ui
                        .add(
                            egui::Button::new(label)
                                .wrap()
                                .fill(self.config.selected_color())
                                .stroke(egui::Stroke::NONE),
                        )
                        .clicked()
                    {
                        self.accept_ai_candidate(candidate, ui.ctx(), false);
                    }
                } else if let Some(message) = &self.message {
                    ui.label(
                        egui::RichText::new(message)
                            .size(self.config.results.font_size)
                            .color(self.config.text_color()),
                    );
                } else if !self.input.trim().is_empty() && self.matches.is_empty() {
                    ui.label(
                        egui::RichText::new("没有匹配结果")
                            .size(self.config.results.font_size)
                            .color(self.config.text_color()),
                    );
                }
                if !self.input.trim().is_empty()
                    && self.ai_available()
                    && !matches!(self.ai_state, AiState::Loading(_) | AiState::Ready(_))
                    && ui.button("使用 DeepSeek 翻译 · Ctrl+Enter").clicked()
                {
                    self.request_ai_translation(ui.ctx());
                }
                if !self.matches.is_empty() {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            for index in visible_range {
                                if self.draw_result(ui, index) {
                                    break;
                                }
                            }
                        });
                }
            },
        );
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }
}

impl UiConfig {
    fn input_size(&self) -> egui::Vec2 {
        egui::vec2(self.input.width, self.input.height)
    }
}

fn reveal_native_window(frame: &eframe::Frame) {
    if let Some(window) = frame.winit_window() {
        window.set_minimized(false);
        window.set_visible(true);
        window.focus_window();
        window.request_redraw();
    }
}

fn voice_input_status(ai_result: bool, method: VoiceInputMethod) -> &'static str {
    match (ai_result, method) {
        (true, VoiceInputMethod::Paste) => "AI 译文已发送 Ctrl+V（目标是否接收需实测）",
        (false, VoiceInputMethod::Paste) => "词库结果已发送 Ctrl+V（目标是否接收需实测）",
        (true, VoiceInputMethod::Unicode) => "AI 译文改用 Unicode 输入（剪贴板不可用）",
        (false, VoiceInputMethod::Unicode) => "词库结果改用 Unicode 输入（剪贴板不可用）",
        (true, VoiceInputMethod::ManualPaste) => "AI 译文已复制；请在 Aion2 按 Ctrl+V",
        (false, VoiceInputMethod::ManualPaste) => "词库结果已复制；请在 Aion2 按 Ctrl+V",
        #[cfg(feature = "tsf-notepad-prototype")]
        (true, VoiceInputMethod::TextService) => "AI 译文已交给记事本文本服务（原型）",
        #[cfg(feature = "tsf-notepad-prototype")]
        (false, VoiceInputMethod::TextService) => "词库结果已交给记事本文本服务（原型）",
    }
}

fn voice_original_status(method: VoiceInputMethod) -> &'static str {
    match method {
        VoiceInputMethod::Paste => "识别原文已发送 Ctrl+V（目标是否接收需实测）",
        VoiceInputMethod::Unicode => "识别原文改用 Unicode 输入（剪贴板不可用）",
        VoiceInputMethod::ManualPaste => "识别原文已复制；请在 Aion2 按 Ctrl+V",
        #[cfg(feature = "tsf-notepad-prototype")]
        VoiceInputMethod::TextService => "识别原文已交给记事本文本服务（原型）",
    }
}

fn search_voice_words<'a>(words: &WordIndex, query: &'a str) -> (Vec<SearchHit>, &'a str) {
    let hits = words.search(query, "", &[]);
    if !hits.is_empty() {
        return (hits, query);
    }
    let cleaned = clean_voice_query(query);
    if cleaned == query || cleaned.is_empty() {
        return (hits, query);
    }
    (words.search(cleaned, "", &[]), cleaned)
}

fn voice_lookup_output<'a>(entry: &'a WordEntry, query: &str) -> &'a str {
    if entry.key.contains(query) && !entry.value.contains(query) {
        &entry.value
    } else {
        &entry.key
    }
}

fn clean_voice_query(text: &str) -> &str {
    text.trim_matches(|character: char| {
        character.is_whitespace()
            || matches!(
                character,
                '。' | '，' | '！' | '？' | '、' | '.' | ',' | '!' | '?' | '"' | '\'' | '“' | '”'
            )
    })
}

fn estimate_text_width(entry: &WordEntry, font_size: f32) -> f32 {
    let units: f32 = entry
        .key
        .chars()
        .chain(std::iter::once(' '))
        .chain(entry.value.chars())
        .map(|character| if character.is_ascii() { 0.58 } else { 1.0 })
        .sum();
    units * font_size + 28.0
}

fn estimate_plain_text_width(text: &str, font_size: f32) -> f32 {
    let units: f32 = text
        .chars()
        .map(|character| if character.is_ascii() { 0.58 } else { 1.0 })
        .sum();
    units * font_size + 70.0
}

/// 只有文本框实际写入空格时才分段，避免输入法用空格选词时提前提交。
fn take_query_separator(input: &mut String) -> bool {
    let end = input.trim_end_matches([' ', '\u{3000}']).len();
    if end == input.len() {
        return false;
    }
    input.truncate(end);
    !input.trim().is_empty()
}

fn highlighted_match_text(
    text: &str,
    query: &str,
    font_size: f32,
    color: egui::Color32,
    highlight_color: egui::Color32,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let normal = egui::TextFormat {
        font_id: egui::FontId::proportional(font_size),
        color,
        ..Default::default()
    };
    if query.is_empty() {
        job.append(text, 0.0, normal);
        return job;
    }

    let mut from = 0;
    while let Some(offset) = text[from..].find(query) {
        let start = from + offset;
        let end = start + query.len();
        job.append(&text[from..start], 0.0, normal.clone());
        let mut highlighted = normal.clone();
        highlighted.color = highlight_color;
        highlighted.background = ui_theme::pale_yellow();
        job.append(&text[start..end], 0.0, highlighted);
        from = end;
    }
    job.append(&text[from..], 0.0, normal);
    job
}

fn configure_fonts(context: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let font_sources = [
        (
            "system_korean",
            &[
                r"C:\Windows\Fonts\malgun.ttf",
                r"C:\Windows\Fonts\gulim.ttc",
                r"C:\Windows\Fonts\batang.ttc",
            ][..],
        ),
        (
            "system_chinese",
            &[
                r"C:\Windows\Fonts\msyh.ttc",
                r"C:\Windows\Fonts\Deng.ttf",
                r"C:\Windows\Fonts\simhei.ttf",
            ][..],
        ),
        (
            "system_japanese",
            &[
                r"C:\Windows\Fonts\YuGothM.ttc",
                r"C:\Windows\Fonts\meiryo.ttc",
                r"C:\Windows\Fonts\msgothic.ttc",
            ][..],
        ),
    ];

    let mut installed = Vec::new();
    for (name, candidates) in font_sources {
        if let Some(bytes) = candidates.iter().find_map(|path| fs::read(path).ok()) {
            fonts
                .font_data
                .insert(name.to_owned(), egui::FontData::from_owned(bytes).into());
            installed.push(name.to_owned());
        }
    }

    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .extend(installed.iter().cloned());
    }

    let editor_font_name = "word_editor_songti";
    let editor_font_candidates = [
        r"C:\Windows\Fonts\simsun.ttc",
        r"C:\Windows\Fonts\simkai.ttf",
    ];
    if let Some(bytes) = editor_font_candidates
        .iter()
        .find_map(|path| fs::read(path).ok())
    {
        fonts.font_data.insert(
            editor_font_name.to_owned(),
            egui::FontData::from_owned(bytes).into(),
        );
        let mut editor_fonts = vec![editor_font_name.to_owned()];
        editor_fonts.extend(installed);
        fonts.families.insert(
            egui::FontFamily::Name(WORD_EDITOR_FONT_FAMILY.into()),
            editor_fonts,
        );
    }
    context.set_fonts(fonts);
}

fn configure_style(context: &egui::Context) {
    context.set_theme(egui::Theme::Light);
    let mut style = (*context.style_of(egui::Theme::Light)).clone();
    style.spacing.item_spacing = egui::Vec2::ZERO;
    style.visuals.window_fill = ui_theme::control_surface();
    style.visuals.panel_fill = egui::Color32::TRANSPARENT;
    context.set_style_of(egui::Theme::Light, style);
}

pub(crate) fn run() -> eframe::Result {
    let defaults = UiConfig::default();
    let initial_size = [defaults.input.width, defaults.input.height];
    let icon = image::load_from_memory(include_bytes!(concat!(
        env!("OUT_DIR"),
        "/to_words_window_256.png"
    )))
    .expect("窗口图标应在构建时生成")
    .to_rgba8();
    let (icon_width, icon_height) = icon.dimensions();
    let viewport = egui::ViewportBuilder::default()
        .with_icon(egui::IconData {
            rgba: icon.into_raw(),
            width: icon_width,
            height: icon_height,
        })
        .with_position([-10_000.0, -10_000.0])
        .with_inner_size(initial_size)
        .with_resizable(false)
        .with_decorations(false)
        .with_transparent(true)
        .with_visible(false)
        .with_window_level(egui::WindowLevel::AlwaysOnTop);

    let options = eframe::NativeOptions {
        viewport,
        centered: false,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };

    eframe::run_native(
        "hotkey_word_match",
        options,
        Box::new(|creation_context| {
            MatchApp::new(&creation_context.egui_ctx)
                .map(|app| Box::new(app) as Box<dyn eframe::App>)
                .map_err(|error| {
                    Box::new(std::io::Error::other(format!("{error:#}")))
                        as Box<dyn std::error::Error + Send + Sync>
                })
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        PopupAnchor, UiConfig, parse_hex_color, parse_hotkeys, popup_position,
        take_query_separator, voice_review_due,
    };
    use eframe::egui;
    use std::time::Duration;

    #[test]
    fn opening_word_editor_can_force_voice_cache_review_before_threshold() {
        assert!(!voice_review_due(1, 1, false, Duration::from_secs(10)));
        assert!(voice_review_due(1, 1, true, Duration::from_secs(10)));
        assert!(!voice_review_due(0, 20, true, Duration::from_secs(10)));
    }

    #[test]
    fn dropdown_background_is_opaque_while_main_panel_stays_transparent() {
        let context = egui::Context::default();
        super::configure_style(&context);
        let visuals = &context.style_of(egui::Theme::Light).visuals;
        assert_eq!(visuals.window_fill.a(), 255);
        assert_eq!(visuals.panel_fill, egui::Color32::TRANSPARENT);
    }

    #[test]
    fn default_input_is_200_by_30_with_twenty_percent_opacity() {
        let config = UiConfig::default();
        assert_eq!(config.input.width, 200.0);
        assert_eq!(config.input.height, 30.0);
        assert_eq!(config.input.opacity, 0.20);
        assert_eq!(config.results.max_visible_results, 8);
        assert!(!config.continuous_input);
        assert!(config.ai_auto_save);
        assert!(!config.ai_polite_mode);
        assert!(config.ocr_auto_translate);
        assert!(config.voice_auto_copy_first);
        assert!(!config.voice_aion2_manual_paste);
        assert!(!config.voice_keep_input);
        assert!(!config.voice_hold_to_talk);
        assert_eq!(config.voice_silence_seconds, 0.6);
        assert_eq!(config.voice_backend, "sensevoice");
        assert_eq!(config.voice_language, "auto");
        assert!(!config.clear_on_focus_return);
        assert!(config.translation_language.is_empty());
        assert_eq!(config.source_language, "auto");
        assert_eq!(config.ui_language, "zh-CN");
    }

    #[test]
    fn removed_windows_speech_backend_falls_back_to_sensevoice() {
        let config = UiConfig {
            voice_backend: "windows".to_string(),
            ..UiConfig::default()
        };
        assert_eq!(config.normalized().voice_backend, "sensevoice");
    }

    #[test]
    fn saved_whisper_backend_remains_available() {
        let config = UiConfig {
            voice_backend: "whisper".to_string(),
            ..UiConfig::default()
        };
        assert_eq!(config.normalized().voice_backend, "whisper");
    }

    #[test]
    fn voice_pause_setting_is_clamped_and_old_configs_use_default() {
        let old: UiConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(old.voice_silence_seconds, 0.6);
        let short = UiConfig {
            voice_silence_seconds: 0.01,
            ..UiConfig::default()
        };
        assert_eq!(short.normalized().voice_silence_seconds, 0.2);
        let long = UiConfig {
            voice_silence_seconds: 9.0,
            ..UiConfig::default()
        };
        assert_eq!(long.normalized().voice_silence_seconds, 3.0);
    }

    #[test]
    fn existing_config_without_ai_auto_save_keeps_default_enabled() {
        let config: UiConfig = serde_json::from_str(r#"{"ai_translation":true}"#).unwrap();
        assert!(config.ai_translation);
        assert!(config.ai_auto_save);
        assert!(!config.ai_polite_mode);
        assert!(config.ocr_auto_translate);
        assert_eq!(config.source_language, "auto");
        assert_eq!(config.ui_language, "zh-CN");
    }

    #[test]
    fn ui_language_is_persisted_without_changing_translation_direction() {
        let config: UiConfig = serde_json::from_str(
            r#"{"ui_language":"ko","source_language":"cn","translation_language":"ja"}"#,
        )
        .unwrap();
        let normalized = config.normalized();
        assert_eq!(normalized.ui_language, "ko");
        assert_eq!(normalized.source_language, "cn");
        assert_eq!(normalized.translation_language, "ja");
        let saved = serde_json::to_value(normalized).unwrap();
        assert_eq!(saved["ui_language"], "ko");

        let invalid: UiConfig = serde_json::from_str(r#"{"ui_language":"xx"}"#).unwrap();
        assert_eq!(invalid.normalized().ui_language, "zh-CN");
    }

    #[test]
    fn former_korean_honorific_setting_migrates_to_general_polite_mode() {
        let config: UiConfig = serde_json::from_str(r#"{"ai_korean_honorific":true}"#).unwrap();
        assert!(config.ai_polite_mode);
        let saved = serde_json::to_value(config).unwrap();
        assert_eq!(saved["ai_polite_mode"], true);
        assert!(saved.get("ai_korean_honorific").is_none());
    }

    #[test]
    fn ocr_auto_translation_can_be_disabled_in_config() {
        let config: UiConfig = serde_json::from_str(r#"{"ocr_auto_translate":false}"#).unwrap();
        assert!(!config.ocr_auto_translate);
    }

    #[test]
    fn older_config_receives_default_voice_options() {
        let config: UiConfig = serde_json::from_str("{}").unwrap();
        assert!(config.voice_auto_copy_first);
        assert!(!config.voice_aion2_manual_paste);
        assert!(!config.voice_keep_input);
        assert!(!config.voice_hold_to_talk);
        assert_eq!(config.voice_silence_seconds, 0.6);
        assert_eq!(config.voice_backend, "sensevoice");
        assert_eq!(config.voice_language, "auto");
        assert_eq!(config.hotkeys.voice, "ctrl+alt+v");
    }

    #[test]
    fn legacy_background_blur_setting_is_ignored() {
        let config: UiConfig = serde_json::from_str(r#"{"background_blur":true}"#).unwrap();
        assert!(
            serde_json::to_value(config)
                .unwrap()
                .get("background_blur")
                .is_none()
        );
    }

    #[test]
    fn voice_query_can_ignore_spoken_sentence_punctuation() {
        assert_eq!(super::clean_voice_query("  “你好。”  "), "你好");
        assert_eq!(super::clean_voice_query("谢谢！"), "谢谢");
    }

    #[test]
    fn voice_query_uses_local_dictionary_before_ai_fallback() {
        use super::{WordIndex, search_voice_words, voice_lookup_output};
        use std::collections::HashMap;

        let words = WordIndex::from_map(HashMap::from([(
            "안녕하세요".to_string(),
            "你好".to_string(),
        )]));
        let (hits, query) = search_voice_words(&words, "你好！");
        assert_eq!(query, "你好");
        assert_eq!(hits.len(), 1);
        assert_eq!(words.get(hits[0].entry_id).unwrap().key, "안녕하세요");
        assert_eq!(
            voice_lookup_output(words.get(hits[0].entry_id).unwrap(), query),
            "안녕하세요"
        );

        let (reverse_hits, reverse_query) = search_voice_words(&words, "안녕하세요");
        assert_eq!(
            voice_lookup_output(words.get(reverse_hits[0].entry_id).unwrap(), reverse_query),
            "你好"
        );

        let (misses, query) = search_voice_words(&words, "未收录");
        assert!(misses.is_empty());
        assert_eq!(query, "未收录");
    }

    #[test]
    fn disabled_ocr_translation_does_not_request_translation() {
        use super::{ocr_translation_target, same_language_no_translation};

        assert!(same_language_no_translation("ko", "ko"));
        assert!(!same_language_no_translation("auto", "ko"));
        assert!(!same_language_no_translation("ko", ""));
        assert_eq!(ocr_translation_target(false, "ko", "cn"), None);
        assert_eq!(ocr_translation_target(true, "ko", "ko"), None);
        assert_eq!(
            ocr_translation_target(true, "auto", "ko").as_deref(),
            Some("cn")
        );
        assert_eq!(
            ocr_translation_target(true, "ja", "ko").as_deref(),
            Some("ja")
        );
    }

    #[test]
    fn unchanged_ocr_selection_keeps_its_recognition_preview() {
        use super::{CaptureRegion, OcrPreview};

        let mut preview = OcrPreview::default();
        let region = CaptureRegion {
            x: 10,
            y: 20,
            width: 120,
            height: 40,
        };
        preview.schedule(region);
        preview.recognized = Some("原文".to_string());
        preview.schedule(region);
        assert_eq!(preview.recognized.as_deref(), Some("原文"));

        preview.schedule(CaptureRegion { x: 11, ..region });
        assert!(preview.recognized.is_none());
    }

    #[test]
    fn parses_configured_hex_color_and_opacity() {
        let color = parse_hex_color("#E8E8E8", 0.20).unwrap();
        let [red, green, blue, alpha] = color.to_srgba_unmultiplied();
        assert!(red.abs_diff(232) <= 2);
        assert!(green.abs_diff(232) <= 2);
        assert!(blue.abs_diff(232) <= 2);
        assert_eq!(alpha, 51);
    }

    #[test]
    fn default_hotkeys_are_distinct_and_valid() {
        let config = UiConfig::default();
        let hotkeys = parse_hotkeys(&config).unwrap();
        assert_ne!(hotkeys.popup, hotkeys.settings);
        assert_ne!(hotkeys.ocr, hotkeys.popup);
        assert_ne!(hotkeys.ocr, hotkeys.settings);
        assert_ne!(hotkeys.voice, hotkeys.popup);
        assert_ne!(hotkeys.voice, hotkeys.settings);
        assert_ne!(hotkeys.voice, hotkeys.ocr);
        assert_eq!(config.hotkeys.popup, "ctrl+alt+enter");
        assert_eq!(config.hotkeys.settings, "ctrl+alt+s");
        assert_eq!(config.hotkeys.clear_composed, "ctrl+backspace");
        assert_eq!(config.hotkeys.ocr, "ctrl+alt+o");
        assert_eq!(config.hotkeys.voice, "ctrl+alt+v");
    }

    #[test]
    fn space_separator_keeps_the_query_until_it_can_be_committed() {
        let mut query = "你好 ".to_string();
        assert!(take_query_separator(&mut query));
        assert_eq!(query, "你好");

        let mut full_width = "안녕하세요　".to_string();
        assert!(take_query_separator(&mut full_width));
        assert_eq!(full_width, "안녕하세요");

        let mut blank = " ".to_string();
        assert!(!take_query_separator(&mut blank));
        assert!(blank.is_empty());

        let mut unchanged = "查询文字".to_string();
        assert!(!take_query_separator(&mut unchanged));
        assert_eq!(unchanged, "查询文字");
    }

    #[test]
    fn older_hotkey_settings_receive_the_new_ocr_default() {
        let config: UiConfig = serde_json::from_str(
            r#"{"hotkeys":{"popup":"ctrl+alt+enter","settings":"ctrl+alt+s","clear_composed":"ctrl+backspace"}}"#,
        )
        .unwrap();
        assert_eq!(config.hotkeys.ocr, "ctrl+alt+o");
        assert!(parse_hotkeys(&config).is_ok());
    }

    #[test]
    fn duplicate_ocr_hotkey_is_rejected() {
        let mut config = UiConfig::default();
        config.hotkeys.ocr = config.hotkeys.settings.clone();
        assert!(parse_hotkeys(&config).is_err());
    }

    #[test]
    fn duplicate_voice_hotkey_is_rejected() {
        let mut config = UiConfig::default();
        config.hotkeys.voice = config.hotkeys.popup.clone();
        assert!(parse_hotkeys(&config).is_err());
    }

    #[test]
    fn mouse_side_button_can_be_bound_and_duplicates_are_rejected() {
        let mut config = UiConfig::default();
        config.hotkeys.popup = "ctrl+MouseX1".to_string();
        config.hotkeys.voice = "MouseX2".to_string();
        let hotkeys = parse_hotkeys(&config).unwrap();
        assert!(hotkeys.popup.mouse().is_some());
        assert!(hotkeys.voice.mouse().is_some());
        assert_eq!(hotkeys.global_keyboard_keys().len(), 2);

        config.hotkeys.ocr = "ctrl+mousex1".to_string();
        assert!(parse_hotkeys(&config).is_err());
    }

    #[test]
    fn popup_is_placed_below_the_text_caret() {
        let anchor = PopupAnchor {
            x: 100.0,
            caret_rect: Some(egui::Rect::from_min_max(
                egui::pos2(100.0, 100.0),
                egui::pos2(102.0, 120.0),
            )),
            fallback_bottom: 748.0,
            work_rect: egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1000.0, 800.0)),
        };
        assert_eq!(
            popup_position(anchor, egui::vec2(200.0, 100.0)),
            egui::pos2(100.0, 128.0)
        );
    }

    #[test]
    fn popup_moves_above_the_caret_when_the_bottom_has_no_room() {
        let anchor = PopupAnchor {
            x: 900.0,
            caret_rect: Some(egui::Rect::from_min_max(
                egui::pos2(900.0, 760.0),
                egui::pos2(902.0, 780.0),
            )),
            fallback_bottom: 748.0,
            work_rect: egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1000.0, 800.0)),
        };
        assert_eq!(
            popup_position(anchor, egui::vec2(200.0, 100.0)),
            egui::pos2(800.0, 652.0)
        );
    }
}
