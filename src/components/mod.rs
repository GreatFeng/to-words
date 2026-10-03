//! 可复用的界面组件入口。
//!
//! 组件按功能区域分为 [`settings`] 设置界面，以及 [`word_library`] 词库相关
//! 界面。组件只负责状态展示与用户交互，查询、合并等业务规则由 `domain` 提供。

pub(crate) mod ocr_selection;
pub(crate) mod settings;
pub(crate) mod word_library;
