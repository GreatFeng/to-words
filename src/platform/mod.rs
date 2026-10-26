//! 操作系统集成能力入口。
//!
//! 当前包含系统托盘、凭据、OCR 和语音识别；后续窗口定位、系统剪贴板或其他 Windows API 能力也应
//! 放在这一层，避免平台相关代码扩散到界面和领域模块。

#[cfg(windows)]
pub(crate) mod caret_position;
pub(crate) mod credentials;
pub(crate) mod extra_mouse_buttons;
pub(crate) mod mouse_hotkey;
pub(crate) mod narrator_voice;
pub(crate) mod ocr;
pub(crate) mod piper_tts;
#[cfg(windows)]
pub(crate) mod streaming_asr;
pub(crate) mod tts_output;
pub(crate) mod voice;
pub(crate) mod voice_aec;
pub(crate) mod voice_identity;
pub(crate) mod voice_metrics;
pub(crate) mod voice_output;
pub(crate) mod voice_path_compat;
pub(crate) mod voice_software_aec;
pub(crate) mod window_monitor;
pub(crate) mod windows_tts;
