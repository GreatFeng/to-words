//! 设置界面及其子页面的本地化文字。
//!
//! 中文是开发时的原文；其他语言的词条放在 `assets/i18n`。词库内容、
//! 翻译方向和语音识别语言不受界面语言影响。

use std::cell::Cell;
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UiLanguage {
    ZhCn,
    En,
    Ko,
    Ja,
}

impl UiLanguage {
    pub(crate) fn from_code(code: &str) -> Self {
        match code {
            "en" => Self::En,
            "ko" => Self::Ko,
            "ja" => Self::Ja,
            _ => Self::ZhCn,
        }
    }

    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::ZhCn => "zh-CN",
            Self::En => "en",
            Self::Ko => "ko",
            Self::Ja => "ja",
        }
    }
}

thread_local! {
    static ACTIVE_LANGUAGE: Cell<UiLanguage> = const { Cell::new(UiLanguage::ZhCn) };
}

pub(crate) fn set_language(code: &str) {
    ACTIVE_LANGUAGE.with(|language| language.set(UiLanguage::from_code(code)));
}

fn catalog(language: UiLanguage) -> Option<&'static HashMap<String, String>> {
    static EN: OnceLock<HashMap<String, String>> = OnceLock::new();
    static KO: OnceLock<HashMap<String, String>> = OnceLock::new();
    static JA: OnceLock<HashMap<String, String>> = OnceLock::new();
    let (slot, content) = match language {
        UiLanguage::ZhCn => return None,
        UiLanguage::En => (&EN, include_str!("../assets/i18n/en.json")),
        UiLanguage::Ko => (&KO, include_str!("../assets/i18n/ko.json")),
        UiLanguage::Ja => (&JA, include_str!("../assets/i18n/ja.json")),
    };
    Some(slot.get_or_init(|| serde_json::from_str(content).expect("valid UI translation catalog")))
}

pub(crate) fn tr(text: &str) -> &str {
    ACTIVE_LANGUAGE.with(|language| {
        catalog(language.get())
            .and_then(|strings| strings.get(text))
            .map(String::as_str)
            .unwrap_or(text)
    })
}

pub(crate) fn message(template: &str, values: &[(&str, &str)]) -> String {
    let mut result = tr(template).to_string();
    for (name, value) in values {
        result = result.replace(&format!("{{{name}}}"), value);
    }
    result
}

pub(crate) fn language_name(code: &str) -> &str {
    // 语言代码保持不变，只本地化设置页下拉列表中的可见名称。
    let name = crate::translation_language::language_label(code);
    tr(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_languages_have_translations() {
        for code in ["en", "ko", "ja"] {
            set_language(code);
            assert_ne!(tr("设置"), "设置");
        }
        set_language("zh-CN");
        assert_eq!(tr("设置"), "设置");
    }

    #[test]
    fn unknown_language_falls_back_to_chinese() {
        assert_eq!(UiLanguage::from_code("unsupported").code(), "zh-CN");
    }

    #[test]
    fn all_catalogs_have_the_same_keys_and_placeholders() {
        let en = catalog(UiLanguage::En).unwrap();
        for language in [UiLanguage::Ko, UiLanguage::Ja] {
            let translated = catalog(language).unwrap();
            assert_eq!(en.len(), translated.len());
            for key in en.keys() {
                let value = translated
                    .get(key)
                    .unwrap_or_else(|| panic!("missing: {key}"));
                for placeholder in key
                    .split('{')
                    .skip(1)
                    .filter_map(|part| part.split('}').next())
                {
                    assert!(value.contains(&format!("{{{placeholder}}}")), "{key}");
                }
            }
        }
        for language in [UiLanguage::En, UiLanguage::Ko, UiLanguage::Ja] {
            let translated = catalog(language).unwrap();
            for option in crate::translation_language::LANGUAGES {
                assert!(
                    translated.contains_key(option.label),
                    "missing language name: {}",
                    option.label
                );
            }
        }
    }
}
