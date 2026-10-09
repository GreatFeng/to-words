//! 与界面和操作系统无关的核心业务逻辑入口。
//!
//! 子模块分别负责连续组合内容、语言与词库文件映射、检索索引和词库合并。
//! 领域层尽量不依赖 egui 或 Windows API，以便独立测试和复用。

pub(crate) mod ai_word_save;
pub(crate) mod continuous_input;
pub(crate) mod shortcut;
pub(crate) mod storage;
pub(crate) mod translation_language;
pub(crate) mod voice_word_cache;
pub(crate) mod word_index;
pub(crate) mod word_merge;
