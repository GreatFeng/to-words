//! 可复用的界面组件入口。
//!
//! 组件按功能区域分为 [`settings`] 设置界面、[`word_library`] 词库编辑、
//! [`ocr_selection`] 截图框选和 [`ocr_translation`] OCR 译文文字预览。
//! 组件只负责状态展示与用户交互，查询、合并等业务规则由 `domain` 提供。

pub(crate) mod ocr_selection;
pub(crate) mod ocr_translation;
pub(crate) mod settings;
pub(crate) mod word_library;
