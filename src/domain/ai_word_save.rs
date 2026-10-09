//! 将用户确认的 AI 译文收录到当前语言词库。

use crate::ai::{AiCandidate, TranslationDirection};
use crate::domain::storage;
use crate::translation_language;
use anyhow::{Result, bail};
#[cfg(test)]
use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

static WORD_LIBRARY_WRITE_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn lock_word_library() -> MutexGuard<'static, ()> {
    WORD_LIBRARY_WRITE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Debug)]
pub(crate) struct ConflictingWordKey;

impl std::fmt::Display for ConflictingWordKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("词库中已有相同 key，但对应的 value 不同；未覆盖原词条")
    }
}

impl std::error::Error for ConflictingWordKey {}

pub(crate) fn save_candidate(candidate: &AiCandidate) -> Result<()> {
    let file_name = translation_language::word_file_name(
        &candidate.source_language_code,
        &candidate.language_code,
    );
    let (key, value) = candidate_pair(candidate)?;
    let _guard = lock_word_library();
    if let Some(existing) = storage::get_word(&file_name, key)? {
        if existing.value != value {
            return Err(ConflictingWordKey.into());
        }
    }
    if storage::is_quarantined(&file_name, key)? {
        return Err(ConflictingWordKey.into());
    }
    storage::insert_word_if_absent(&file_name, key, value, &candidate.pronunciation, "ai")?;
    Ok(())
}

fn candidate_pair(candidate: &AiCandidate) -> Result<(&str, &str)> {
    let (key, value) = match candidate.direction {
        TranslationDirection::Forward => (candidate.translated.trim(), candidate.source.trim()),
        TranslationDirection::Reverse => (candidate.source.trim(), candidate.translated.trim()),
    };
    if key.is_empty() || value.is_empty() {
        bail!("译文或原文不能为空");
    }
    Ok((key, value))
}

#[cfg(test)]
fn insert_candidate(words: &mut BTreeMap<String, String>, candidate: &AiCandidate) -> Result<bool> {
    let (key, value) = candidate_pair(candidate)?;
    if let Some(existing) = words.get(key) {
        if existing == value {
            return Ok(false);
        }
        return Err(ConflictingWordKey.into());
    }
    words.insert(key.to_owned(), value.to_owned());
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::insert_candidate;
    use crate::ai::{AiCandidate, TranslationDirection};
    use std::collections::BTreeMap;

    #[test]
    fn existing_user_entry_is_never_overwritten() {
        let candidate = AiCandidate {
            source: "你好".into(),
            translated: "안녕하세요".into(),
            pronunciation: String::new(),
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
    fn reverse_translation_is_saved_as_target_language_key() {
        let candidate = AiCandidate {
            source: "안녕하세요".into(),
            translated: "你好".into(),
            pronunciation: String::new(),
            source_language_code: "cn".into(),
            language_code: "ko".into(),
            direction: TranslationDirection::Reverse,
        };
        let mut words = BTreeMap::new();
        assert!(insert_candidate(&mut words, &candidate).unwrap());
        assert_eq!(words["안녕하세요"], "你好");
    }
}
