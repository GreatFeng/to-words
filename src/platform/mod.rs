//! 操作系统集成能力入口。
//!
//! 当前包含系统托盘、凭据、OCR 和语音识别；后续窗口定位、系统剪贴板或其他 Windows API 能力也应
//! 放在这一层，避免平台相关代码扩散到界面和领域模块。

pub(crate) mod credentials;
pub(crate) mod extra_mouse_buttons;
pub(crate) mod mouse_hotkey;
pub(crate) mod ocr;
pub(crate) mod system_tray;
pub(crate) mod voice;
pub(crate) mod voice_metrics;
pub(crate) mod voice_output;
pub(crate) mod voice_overlay_window;
