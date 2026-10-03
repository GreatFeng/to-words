//! 将用户确认的 AI 译文收录到当前语言词库。

use crate::ai::{AiCandidate, TranslationDirection};
use crate::{prepare_word_library_file, translation_language};
use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

pub(crate) fn save_candidate(candidate: &AiCandidate) -> Result<()> {
    let file_name = translation_language::word_file_name(
        &candidate.source_language_code,
        &candidate.language_code,
    );
    let path = prepare_word_library_file(&file_name)?;
    save_candidate_to_path(&path, candidate)
}

fn save_candidate_to_path(path: &Path, candidate: &AiCandidate) -> Result<()> {
    let content =
        fs::read_to_string(path).with_context(|| format!("无法读取词库：{}", path.display()))?;
    let mut words: BTreeMap<String, String> = serde_json::from_str(&content)
        .with_context(|| format!("词库 JSON 格式错误：{}", path.display()))?;
    if !insert_candidate(&mut words, candidate)? {
        return Ok(());
    }
    let formatted = serde_json::to_string_pretty(&words).context("无法格式化词库")?;
    fs::write(path, format!("{formatted}\n"))
        .with_context(|| format!("无法保存词库：{}", path.display()))?;
    Ok(())
}

fn insert_candidate(words: &mut BTreeMap<String, String>, candidate: &AiCandidate) -> Result<bool> {
    let (key, value) = match candidate.direction {
        TranslationDirection::Forward => (candidate.translated.trim(), candidate.source.trim()),
        TranslationDirection::Reverse => (candidate.source.trim(), candidate.translated.trim()),
    };
    if key.is_empty() || value.is_empty() {
        bail!("译文或原文不能为空");
    }
    if let Some(existing) = words.get(key) {
        if existing == value {
            return Ok(false);
        }
        bail!("词库中已有相同 key，但对应的 value 不同；未覆盖原词条");
    }
    words.insert(key.to_owned(), value.to_owned());
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::{insert_candidate, save_candidate_to_path};
    use crate::ai::{AiCandidate, TranslationDirection};
    use std::collections::BTreeMap;
    use std::fs;

    #[test]
    fn existing_user_entry_is_never_overwritten() {
        let candidate = AiCandidate {
            source: "你好".into(),
            translated: "안녕하세요".into(),
            source_language_code: "cn".into(),
            language_code: "ko".into(),
            direction: TranslationDirection::Forward,
        };
        let mut words = BTreeMap::from([("안녕하세요".into(), "已有解释".into())]);
        assert!(insert_candidate(&mut words, &candidate).is_err());
        assert_eq!(words["안녕하세요"], "已有解释");
        words.clear();
        assert!(insert_candidate(&mut words, &candidate).unwrap());
        assert!(!insert_candidate(&mut words, &candidate).unwrap());
    }

    #[test]
    fn confirmed_translation_is_written_as_key_and_source_as_value() {
        let file = std::env::temp_dir().join(format!(
            "to_words_ai_save_{}_{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&file, "{\"기존\":\"原有\"}").unwrap();
        let candidate = AiCandidate {
            source: "你好".into(),
            translated: "안녕하세요".into(),
            source_language_code: "cn".into(),
            language_code: "ko".into(),
            direction: TranslationDirection::Forward,
        };
        save_candidate_to_path(&file, &candidate).unwrap();
        let stored: BTreeMap<String, String> =
            serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(stored["안녕하세요"], "你好");
        assert_eq!(stored["기존"], "原有");
        assert!(!file.with_extension("json.bak").exists());
        fs::remove_file(file).unwrap();
    }

    #[test]
    fn reverse_translation_is_saved_as_target_language_key() {
        let candidate = AiCandidate {
            source: "안녕하세요".into(),
            translated: "你好".into(),
            source_language_code: "cn".into(),
            language_code: "ko".into(),
            direction: TranslationDirection::Reverse,
        };
        let mut words = BTreeMap::new();
        assert!(insert_candidate(&mut words, &candidate).unwrap());
        assert_eq!(words["안녕하세요"], "你好");
    }
}
