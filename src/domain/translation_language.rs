//! 转译语言与词库文件名映射。
//!
//! `LANGUAGES` 定义设置页面支持的语言选项；辅助函数负责规范化语言代码、生成
//! 对应的 `user_words_<source>_<target>.json` 文件名，并为界面返回可读标签。

pub(crate) struct TranslationLanguage {
    pub(crate) code: &'static str,
    pub(crate) label: &'static str,
}

pub(crate) const LANGUAGES: &[TranslationLanguage] = &[
    TranslationLanguage {
        code: "cn",
        label: "简体中文",
    },
    TranslationLanguage {
        code: "ko",
        label: "韩语（韩国）",
    },
    TranslationLanguage {
        code: "en",
        label: "英语",
    },
    TranslationLanguage {
        code: "ja",
        label: "日语（日本）",
    },
    TranslationLanguage {
        code: "zh_tw",
        label: "繁体中文",
    },
    TranslationLanguage {
        code: "fr",
        label: "法语",
    },
    TranslationLanguage {
        code: "de",
        label: "德语",
    },
    TranslationLanguage {
        code: "es",
        label: "西班牙语",
    },
    TranslationLanguage {
        code: "pt",
        label: "葡萄牙语",
    },
    TranslationLanguage {
        code: "it",
        label: "意大利语",
    },
    TranslationLanguage {
        code: "ru",
        label: "俄语",
    },
    TranslationLanguage {
        code: "uk",
        label: "乌克兰语",
    },
    TranslationLanguage {
        code: "ar",
        label: "阿拉伯语",
    },
    TranslationLanguage {
        code: "he",
        label: "希伯来语",
    },
    TranslationLanguage {
        code: "fa",
        label: "波斯语",
    },
    TranslationLanguage {
        code: "hi",
        label: "印地语",
    },
    TranslationLanguage {
        code: "ur",
        label: "乌尔都语",
    },
    TranslationLanguage {
        code: "bn",
        label: "孟加拉语",
    },
    TranslationLanguage {
        code: "ta",
        label: "泰米尔语",
    },
    TranslationLanguage {
        code: "te",
        label: "泰卢固语",
    },
    TranslationLanguage {
        code: "th",
        label: "泰语",
    },
    TranslationLanguage {
        code: "vi",
        label: "越南语",
    },
    TranslationLanguage {
        code: "id",
        label: "印度尼西亚语",
    },
    TranslationLanguage {
        code: "ms",
        label: "马来语",
    },
    TranslationLanguage {
        code: "fil",
        label: "菲律宾语",
    },
    TranslationLanguage {
        code: "tr",
        label: "土耳其语",
    },
    TranslationLanguage {
        code: "pl",
        label: "波兰语",
    },
    TranslationLanguage {
        code: "nl",
        label: "荷兰语",
    },
    TranslationLanguage {
        code: "sv",
        label: "瑞典语",
    },
    TranslationLanguage {
        code: "no",
        label: "挪威语",
    },
    TranslationLanguage {
        code: "da",
        label: "丹麦语",
    },
    TranslationLanguage {
        code: "fi",
        label: "芬兰语",
    },
    TranslationLanguage {
        code: "cs",
        label: "捷克语",
    },
    TranslationLanguage {
        code: "sk",
        label: "斯洛伐克语",
    },
    TranslationLanguage {
        code: "hu",
        label: "匈牙利语",
    },
    TranslationLanguage {
        code: "ro",
        label: "罗马尼亚语",
    },
    TranslationLanguage {
        code: "bg",
        label: "保加利亚语",
    },
    TranslationLanguage {
        code: "el",
        label: "希腊语",
    },
    TranslationLanguage {
        code: "sw",
        label: "斯瓦希里语",
    },
    TranslationLanguage {
        code: "km",
        label: "高棉语",
    },
    TranslationLanguage {
        code: "lo",
        label: "老挝语",
    },
    TranslationLanguage {
        code: "my",
        label: "缅甸语",
    },
    TranslationLanguage {
        code: "mn",
        label: "蒙古语",
    },
    TranslationLanguage {
        code: "kk",
        label: "哈萨克语",
    },
    TranslationLanguage {
        code: "uz",
        label: "乌兹别克语",
    },
];

pub(crate) fn language_label(code: &str) -> &'static str {
    LANGUAGES
        .iter()
        .find(|language| language.code == code)
        .map_or("未选择（默认词库）", |language| language.label)
}

pub(crate) fn word_file_name(source: &str, target: &str) -> String {
    if target.is_empty() {
        "user_words.json".to_string()
    } else if is_supported(source) && is_supported(target) {
        format!("user_words_{source}_{target}.json")
    } else {
        "user_words.json".to_string()
    }
}

fn is_supported(code: &str) -> bool {
    LANGUAGES.iter().any(|language| language.code == code)
}

pub(crate) fn normalize_language_code(code: &str) -> String {
    LANGUAGES
        .iter()
        .find(|language| language.code.eq_ignore_ascii_case(code.trim()))
        .map_or_else(String::new, |language| language.code.to_string())
}

pub(crate) fn normalize_source_code(code: &str) -> String {
    if code.trim().eq_ignore_ascii_case("auto") {
        "auto".to_string()
    } else {
        let code = normalize_language_code(code);
        if code.is_empty() {
            "auto".to_string()
        } else {
            code
        }
    }
}

pub(crate) fn same_language_pair<'a>(source: &'a str, target: &str) -> Option<&'a str> {
    (!target.is_empty() && source != "auto" && source == target).then_some(source)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WritingSystem {
    Latin,
    Han,
    Kana,
    Hangul,
    Cyrillic,
    Arabic,
    Hebrew,
    Devanagari,
    Bengali,
    Tamil,
    Telugu,
    Thai,
    Khmer,
    Lao,
    Myanmar,
    Greek,
    Other,
}

fn writing_system(ch: char) -> WritingSystem {
    match ch as u32 {
        0x0041..=0x007a | 0x00c0..=0x024f | 0x1e00..=0x1eff | 0x2c60..=0x2c7f | 0xa720..=0xa7ff => {
            WritingSystem::Latin
        }
        0x3400..=0x9fff | 0xf900..=0xfaff => WritingSystem::Han,
        0x3040..=0x30ff | 0x31f0..=0x31ff | 0xff66..=0xff9f => WritingSystem::Kana,
        0x1100..=0x11ff | 0x3130..=0x318f | 0xac00..=0xd7af => WritingSystem::Hangul,
        0x0400..=0x052f => WritingSystem::Cyrillic,
        0x0600..=0x06ff | 0x0750..=0x077f | 0x08a0..=0x08ff => WritingSystem::Arabic,
        0x0590..=0x05ff => WritingSystem::Hebrew,
        0x0900..=0x097f => WritingSystem::Devanagari,
        0x0980..=0x09ff => WritingSystem::Bengali,
        0x0b80..=0x0bff => WritingSystem::Tamil,
        0x0c00..=0x0c7f => WritingSystem::Telugu,
        0x0e00..=0x0e7f => WritingSystem::Thai,
        0x1780..=0x17ff => WritingSystem::Khmer,
        0x0e80..=0x0eff => WritingSystem::Lao,
        0x1000..=0x109f => WritingSystem::Myanmar,
        0x0370..=0x03ff | 0x1f00..=0x1fff => WritingSystem::Greek,
        _ => WritingSystem::Other,
    }
}

fn expected_writing_system(language: &str) -> Option<WritingSystem> {
    match language {
        "cn" | "zh_tw" => Some(WritingSystem::Han),
        "ja" => Some(WritingSystem::Kana),
        "ko" => Some(WritingSystem::Hangul),
        "ru" | "uk" | "bg" | "mn" | "kk" => Some(WritingSystem::Cyrillic),
        "ar" | "fa" | "ur" => Some(WritingSystem::Arabic),
        "he" => Some(WritingSystem::Hebrew),
        "hi" => Some(WritingSystem::Devanagari),
        "bn" => Some(WritingSystem::Bengali),
        "ta" => Some(WritingSystem::Tamil),
        "te" => Some(WritingSystem::Telugu),
        "th" => Some(WritingSystem::Thai),
        "km" => Some(WritingSystem::Khmer),
        "lo" => Some(WritingSystem::Lao),
        "my" => Some(WritingSystem::Myanmar),
        "el" => Some(WritingSystem::Greek),
        code if is_supported(code) => Some(WritingSystem::Latin),
        _ => None,
    }
}

/// Reject clearly different writing systems in same-language mode. Shared
/// scripts (for example English/French or Japanese/Chinese kanji) cannot be
/// conclusively distinguished from a short text alone.
pub(crate) fn accepts_same_language_text(text: &str, language: &str) -> bool {
    let Some(expected) = expected_writing_system(language) else {
        return true;
    };
    let letters: Vec<_> = text
        .chars()
        .filter(|ch| ch.is_alphabetic())
        .map(writing_system)
        .collect();
    if letters.is_empty() {
        return true; // Numbers and punctuation do not identify a language.
    }
    let count = |system| letters.iter().filter(|&&item| item == system).count();
    let primary = count(expected);
    let han = count(WritingSystem::Han);
    let kana = count(WritingSystem::Kana);
    let hangul = count(WritingSystem::Hangul);
    let allowed = |system| {
        system == expected
            || (expected != WritingSystem::Latin && system == WritingSystem::Latin)
            || (language == "ja" && system == WritingSystem::Han)
            || (language == "ko" && system == WritingSystem::Han)
    };
    if letters.iter().copied().any(|system| !allowed(system)) {
        return false;
    }
    match language {
        "ja" => (kana > 0 || han > 0) && hangul == 0,
        "ko" => hangul > 0 && hangul >= han,
        _ if expected == WritingSystem::Latin => {
            if primary == 0 {
                return false;
            }
            // Long shared-script sentences can be checked more confidently;
            // short gaming terms must not be rejected by unreliable guessing.
            if primary >= 24
                && let Some(detected) = detect_source_language(text)
                && expected_writing_system(detected) == Some(WritingSystem::Latin)
            {
                detected == language
            } else {
                true
            }
        }
        _ => primary > 0,
    }
}

/// 短输入先按文字系统判断；拉丁文字要有足够长度和可信度才切换词库。
/// 无法确定时返回 None，调用方沿用当前词库，避免逐字输入时频繁创建文件。
pub(crate) fn detect_source_language(text: &str) -> Option<&'static str> {
    if text.chars().any(|c| ('\u{ac00}'..='\u{d7af}').contains(&c)) {
        return Some("ko");
    }
    if text.chars().any(|c| ('\u{3040}'..='\u{30ff}').contains(&c)) {
        return Some("ja");
    }
    if text.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)) {
        return Some("cn");
    }
    let letter_count = text.chars().filter(|c| c.is_alphabetic()).count();
    let info = whatlang::detect(text)?;
    if info.script() == whatlang::Script::Latin && letter_count < 12 {
        // 短拉丁词缺少足够特征；常见纯 ASCII 输入先按英语处理。
        return (letter_count >= 4 && text.is_ascii()).then_some("en");
    }
    if !info.is_reliable() {
        return None;
    }
    let code = match info.lang().code() {
        "cmn" => "cn",
        "kor" => "ko",
        "jpn" => "ja",
        "eng" => "en",
        "fra" => "fr",
        "deu" => "de",
        "spa" => "es",
        "por" => "pt",
        "ita" => "it",
        "rus" => "ru",
        "ukr" => "uk",
        "arb" => "ar",
        "heb" => "he",
        "pes" => "fa",
        "hin" => "hi",
        "urd" => "ur",
        "ben" => "bn",
        "tam" => "ta",
        "tel" => "te",
        "tha" => "th",
        "vie" => "vi",
        "ind" => "id",
        "msa" => "ms",
        "tgl" => "fil",
        "tur" => "tr",
        "pol" => "pl",
        "nld" => "nl",
        "swe" => "sv",
        "nob" | "nno" => "no",
        "dan" => "da",
        "fin" => "fi",
        "ces" => "cs",
        "slk" => "sk",
        "hun" => "hu",
        "ron" => "ro",
        "bul" => "bg",
        "ell" => "el",
        "swa" => "sw",
        "khm" => "km",
        "lao" => "lo",
        "mya" => "my",
        "mon" => "mn",
        "kaz" => "kk",
        "uzb" => "uz",
        _ => return None,
    };
    Some(code)
}

#[cfg(test)]
mod tests {
    use super::{
        accepts_same_language_text, detect_source_language, normalize_language_code,
        same_language_pair, word_file_name,
    };

    #[test]
    fn same_language_requires_an_explicit_nonempty_pair() {
        assert_eq!(same_language_pair("en", "en"), Some("en"));
        assert_eq!(same_language_pair("ja", "ja"), Some("ja"));
        assert_eq!(same_language_pair("auto", "en"), None);
        assert_eq!(same_language_pair("en", ""), None);
    }

    #[test]
    fn same_language_text_rejects_other_writing_systems() {
        assert!(accepts_same_language_text("Hello, world!", "en"));
        assert!(accepts_same_language_text(
            "This is a simple English sentence spoken clearly in a quiet room.",
            "en"
        ));
        assert!(!accepts_same_language_text(
            "Bonjour, je voudrais savoir comment vous allez aujourd'hui et demain.",
            "en"
        ));
        assert!(!accepts_same_language_text("你好", "en"));
        assert!(!accepts_same_language_text("こんにちは", "en"));
        assert!(accepts_same_language_text("こんにちは、東京", "ja"));
        assert!(accepts_same_language_text("東京", "ja"));
        assert!(!accepts_same_language_text("hello", "ja"));
        assert!(!accepts_same_language_text("안녕하세요", "ja"));
        assert!(accepts_same_language_text("안녕하세요", "ko"));
        assert!(!accepts_same_language_text("你好", "ko"));
        assert!(accepts_same_language_text("123!", "ko"));
    }

    #[test]
    fn korean_uses_the_cn_to_ko_dictionary() {
        assert_eq!(word_file_name("cn", "ko"), "user_words_cn_ko.json");
        assert_eq!(word_file_name("en", "ko"), "user_words_en_ko.json");
    }

    #[test]
    fn empty_or_unknown_language_uses_the_default_dictionary() {
        assert_eq!(word_file_name("cn", ""), "user_words.json");
        assert_eq!(word_file_name("unknown", "ko"), "user_words.json");
        assert_eq!(normalize_language_code("unknown"), "");
    }

    #[test]
    fn detects_east_asian_scripts_and_defers_short_latin_input() {
        assert_eq!(detect_source_language("你好"), Some("cn"));
        assert_eq!(detect_source_language("안녕하세요"), Some("ko"));
        assert_eq!(detect_source_language("こんにちは"), Some("ja"));
        assert_eq!(detect_source_language("hi"), None);
        assert_eq!(detect_source_language("hello"), Some("en"));
    }
}
