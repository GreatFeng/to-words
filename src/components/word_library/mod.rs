//! 词库类界面组件入口。
//!
//! [`editor`] 提供结构化表格和 JSON 高级编辑；[`merge_panel`] 负责选择本地
//! 词库并执行合并。二者均在当前设置窗口中渲染，不创建独立原生窗口。

pub(crate) mod editor;
pub(crate) mod merge_panel;
