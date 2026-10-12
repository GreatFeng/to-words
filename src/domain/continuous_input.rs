//! 连续输入组合模型。
//!
//! `ContinuousInput` 保存用户已经确认的 key/value 项，并同步生成用于剪贴板的
//! key 文本与用于界面展示的说明文本。其方法负责追加、撤销、全部清空以及读取
//! 当前组合结果，不包含任何 UI 或剪贴板系统调用。

#[derive(Default)]
pub(crate) struct ContinuousInput {
    items: Vec<ComposedItem>,
    clipboard_text: String,
    display_text: String,
}

struct ComposedItem {
    key: String,
    value: String,
    library: Option<String>,
}

impl ContinuousInput {
    pub(crate) fn clear(&mut self) {
        self.items.clear();
        self.rebuild_text();
    }

    pub(crate) fn append(&mut self, key: &str, value: &str) -> &str {
        self.append_with_library(key, value, None)
    }

    pub(crate) fn append_word(&mut self, key: &str, value: &str, library: &str) -> &str {
        self.append_with_library(key, value, Some(library))
    }

    fn append_with_library(&mut self, key: &str, value: &str, library: Option<&str>) -> &str {
        let key = key.trim();
        if !key.is_empty() {
            self.items.push(ComposedItem {
                key: key.to_owned(),
                value: value.trim().to_owned(),
                library: library.map(str::to_owned),
            });
            self.rebuild_text();
        }
        &self.clipboard_text
    }

    pub(crate) fn undo(&mut self) -> &str {
        self.items.pop();
        self.rebuild_text();
        &self.clipboard_text
    }

    pub(crate) fn clipboard_text(&self) -> &str {
        &self.clipboard_text
    }

    pub(crate) fn display_text(&self) -> &str {
        &self.display_text
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub(crate) fn used_words(&self) -> Vec<(String, String)> {
        self.items
            .iter()
            .filter_map(|item| {
                item.library
                    .as_ref()
                    .map(|library| (library.clone(), item.key.clone()))
            })
            .collect()
    }

    fn rebuild_text(&mut self) {
        self.clipboard_text = self
            .items
            .iter()
            .map(|item| item.key.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        self.display_text = self
            .items
            .iter()
            .map(|item| {
                if item.value.is_empty() {
                    item.key.clone()
                } else {
                    format!("{}（{}）", item.key, item.value)
                }
            })
            .collect::<Vec<_>>()
            .join("　");
    }
}

#[cfg(test)]
mod tests {
    use super::ContinuousInput;

    #[test]
    fn composes_without_a_trailing_space_and_supports_undo() {
        let mut input = ContinuousInput::default();
        assert_eq!(input.append("안녕하세요", "你好"), "안녕하세요");
        assert_eq!(input.append("감사합니다", "谢谢"), "안녕하세요 감사합니다");
        assert_eq!(
            input.display_text(),
            "안녕하세요（你好）　감사합니다（谢谢）"
        );
        assert_eq!(input.undo(), "안녕하세요");
        input.clear();
        assert!(input.is_empty());
        assert_eq!(input.clipboard_text(), "");
    }

    #[test]
    fn direct_same_language_text_has_no_empty_parentheses() {
        let mut input = ContinuousInput::default();
        input.append("Hello", "");
        assert_eq!(input.display_text(), "Hello");
    }
}
