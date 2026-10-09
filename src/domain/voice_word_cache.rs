//! 语音 AI 译文入库前的持久化缓存、规则过滤与定时审核。
//!
//! 缓存不含录音或 API Key；识别与即时输出不等待这里的网络审核。

use crate::ai::{self, AiCandidate, VoiceReviewCase, VoiceVerdict};
use crate::domain::ai_word_save::{self, ConflictingWordKey};
use crate::domain::storage;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) const REVIEW_INTERVAL: Duration = Duration::from_secs(60 * 60);
pub(crate) const REVIEW_THRESHOLD: usize = 20;
const REVIEW_BATCH: usize = 10;
const RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const DUPLICATE_WINDOW: Duration = Duration::from_secs(2 * 60);
const CONTEXT_WINDOW: Duration = Duration::from_secs(5 * 60);
const MAX_RECORDS: usize = 1000;

#[derive(Clone)]
pub(crate) struct VoiceCacheRequest {
    pub(crate) candidate: AiCandidate,
    pub(crate) session_id: u64,
    pub(crate) previous: Option<String>,
    pub(crate) next: Option<String>,
}

#[derive(Default, Deserialize, Serialize)]
struct CacheFile {
    records: Vec<CacheRecord>,
}

#[derive(Deserialize, Serialize)]
struct CacheRecord {
    candidate: AiCandidate,
    session_id: u64,
    created_at: u64,
    previous: Option<String>,
    next: Option<String>,
    /// 无法确定的词条仅在邻句上下文变化后重新审核。
    uncertain_context: Option<String>,
    #[serde(default)]
    uncertain_at: Option<u64>,
}

pub(crate) struct VoiceWordCache {
    path: PathBuf,
    file: CacheFile,
    recent: VecDeque<(String, u64)>,
}

impl VoiceWordCache {
    pub(crate) fn load(path: PathBuf) -> Result<Self> {
        let file = if let Some(content) = storage::load_voice_cache_at(&path)? {
            serde_json::from_str(&content)
                .with_context(|| format!("语音词库缓存格式错误：{}", path.display()))?
        } else {
            CacheFile::default()
        };
        let recent = file
            .records
            .iter()
            .map(|record: &CacheRecord| {
                (normalized_text(&record.candidate.source), record.created_at)
            })
            .collect();
        Ok(Self { path, file, recent })
    }

    pub(crate) fn pending_len(&self) -> usize {
        self.file.records.len()
    }

    pub(crate) fn reviewable_len(&self) -> usize {
        self.file
            .records
            .iter()
            .enumerate()
            .filter(|(index, record)| {
                record.uncertain_context.as_ref() != Some(&self.context_for(*index).2)
            })
            .count()
    }

    /// 返回 false 表示规则直接丢弃，且不会修改已有缓存。
    pub(crate) fn enqueue(&mut self, request: VoiceCacheRequest) -> Result<bool> {
        let now = now_secs();
        let source = request.candidate.source.trim();
        let translated = request.candidate.translated.trim();
        if is_obviously_bad(source) || is_obviously_bad(translated) {
            return Ok(false);
        }
        self.prune_recent(now);
        let normalized = normalized_text(source);
        if self.recent.iter().any(|(text, at)| {
            text == &normalized && now.saturating_sub(*at) <= DUPLICATE_WINDOW.as_secs()
        }) {
            return Ok(false);
        }
        if self.file.records.iter().any(|record| {
            record.candidate.source == source
                && record.candidate.translated == translated
                && record.candidate.language_code == request.candidate.language_code
                && record.candidate.source_language_code == request.candidate.source_language_code
        }) {
            return Ok(false);
        }
        if self.file.records.len() >= MAX_RECORDS {
            bail!("语音词库缓存已满（{MAX_RECORDS} 条），未收录新词条");
        }
        self.file.records.push(CacheRecord {
            candidate: request.candidate,
            session_id: request.session_id,
            created_at: now,
            previous: request.previous.map(short_context),
            next: request.next.map(short_context),
            uncertain_context: None,
            uncertain_at: None,
        });
        self.persist()?;
        self.recent.push_back((normalized, now));
        Ok(true)
    }

    pub(crate) fn expire_uncertain(&mut self) -> Result<usize> {
        let now = now_secs();
        let expired: Vec<_> = self
            .file
            .records
            .iter()
            .enumerate()
            .filter_map(|(index, record)| {
                (record.uncertain_context.is_some()
                    && now.saturating_sub(record.uncertain_at.unwrap_or(record.created_at))
                        >= RETENTION.as_secs()
                    && record.uncertain_context.as_ref() == Some(&self.context_for(index).2))
                .then_some(index)
            })
            .collect();
        let removed = expired.len();
        for index in expired.into_iter().rev() {
            self.file.records.remove(index);
        }
        if removed > 0 {
            self.persist()?;
        }
        Ok(removed)
    }

    /// 每次至多审核 10 条，网络失败时不删除缓存；正常结果只有成功写入词库后才移除。
    pub(crate) fn review_once(&mut self, api_key: &str) -> Result<Vec<AiCandidate>> {
        let selected = self.review_cases();
        if selected.is_empty() {
            return Ok(Vec::new());
        }
        let cases: Vec<_> = selected.iter().map(|(_, case, _)| case.clone()).collect();
        let verdicts = ai::review_voice_cases(&cases, api_key)?;
        self.apply_reviews(selected, verdicts, ai_word_save::save_candidate)
    }

    fn apply_reviews(
        &mut self,
        selected: Vec<(usize, VoiceReviewCase, String)>,
        verdicts: Vec<VoiceVerdict>,
        mut save: impl FnMut(&AiCandidate) -> Result<()>,
    ) -> Result<Vec<AiCandidate>> {
        let mut saved = Vec::new();
        let mut completed = Vec::new();
        for ((index, _, context), verdict) in selected.into_iter().zip(verdicts) {
            match verdict {
                VoiceVerdict::Normal => {
                    let candidate = self.file.records[index].candidate.clone();
                    match save(&candidate) {
                        Ok(()) => {
                            saved.push(candidate);
                            completed.push(index);
                        }
                        Err(error) if error.is::<ConflictingWordKey>() => {
                            // 用户已修改同名 key，不覆盖；这条候选无法再安全入库。
                            completed.push(index);
                        }
                        Err(error) => {
                            // 保留写盘失败的条目，后续可重试。
                            eprintln!("语音词库审核通过但写入失败：{error:#}");
                        }
                    }
                }
                VoiceVerdict::Error => completed.push(index),
                VoiceVerdict::Uncertain => {
                    // 新邻句带来了新依据，重新开始计算保留期限。
                    if self.file.records[index].uncertain_context.as_ref() != Some(&context) {
                        self.file.records[index].uncertain_at = Some(now_secs());
                    }
                    self.file.records[index].uncertain_context = Some(context);
                }
            }
        }
        for index in completed.into_iter().rev() {
            self.file.records.remove(index);
        }
        self.persist()?;
        Ok(saved)
    }

    fn review_cases(&self) -> Vec<(usize, VoiceReviewCase, String)> {
        let mut chosen = Vec::new();
        for (index, record) in self.file.records.iter().enumerate() {
            let (previous, next, context) = self.context_for(index);
            if record.uncertain_context.as_ref() == Some(&context) {
                continue;
            }
            let id = chosen.len();
            chosen.push((
                index,
                VoiceReviewCase {
                    id,
                    source: record.candidate.source.clone(),
                    translated: record.candidate.translated.clone(),
                    previous,
                    next,
                },
                context,
            ));
            if chosen.len() >= REVIEW_BATCH {
                break;
            }
        }
        chosen
    }

    fn context_for(&self, index: usize) -> (Option<String>, Option<String>, String) {
        let record = &self.file.records[index];
        let previous = record
            .previous
            .clone()
            .or_else(|| self.neighbor(index, false));
        let next = record.next.clone().or_else(|| self.neighbor(index, true));
        let context = serde_json::to_string(&(&previous, &next)).unwrap_or_default();
        (previous, next, context)
    }

    fn neighbor(&self, index: usize, forward: bool) -> Option<String> {
        let record = self.file.records.get(index)?;
        let neighbor = if forward {
            self.file.records.get(index + 1)?
        } else {
            self.file.records.get(index.checked_sub(1)?)?
        };
        (neighbor.session_id == record.session_id
            && record.created_at.abs_diff(neighbor.created_at) <= CONTEXT_WINDOW.as_secs())
        .then(|| short_context(neighbor.candidate.source.clone()))
    }

    fn persist(&self) -> Result<()> {
        let content = serde_json::to_string(&self.file).context("无法序列化语音词库缓存")?;
        storage::save_voice_cache_at(&self.path, &content)
            .with_context(|| format!("无法更新语音词库缓存：{}", self.path.display()))
    }

    fn prune_recent(&mut self, now: u64) {
        while self
            .recent
            .front()
            .is_some_and(|(_, at)| now.saturating_sub(*at) > DUPLICATE_WINDOW.as_secs())
        {
            self.recent.pop_front();
        }
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn normalized_text(text: &str) -> String {
    text.split_whitespace().collect::<String>().to_lowercase()
}

fn short_context(text: String) -> String {
    text.chars().take(160).collect()
}

pub(crate) fn is_obviously_bad(text: &str) -> bool {
    let text = text.trim();
    let lowered = text.to_ascii_lowercase();
    if text.is_empty()
        || text.chars().count() > 500
        || [
            "[blank_audio]",
            "[no_speech]",
            "[low_confidence]",
            "<|nospeech|>",
            "<|no_speech|>",
            "<|low_confidence|>",
        ]
        .iter()
        .any(|marker| lowered.contains(marker))
    {
        return true;
    }
    let chars: Vec<_> = text.chars().filter(|ch| !ch.is_whitespace()).collect();
    let letters = chars.iter().filter(|ch| ch.is_alphanumeric()).count();
    if letters == 0 || letters * 4 < chars.len() {
        return true;
    }
    let mut run = 0;
    let mut longest = 0;
    let mut previous = None;
    for ch in chars.iter().copied() {
        run = if previous == Some(ch) { run + 1 } else { 1 };
        longest = longest.max(run);
        previous = Some(ch);
    }
    longest >= 6 && longest * 4 >= chars.len() * 3
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::TranslationDirection;
    use std::fs;

    fn candidate(source: &str) -> AiCandidate {
        AiCandidate {
            source: source.into(),
            translated: "안녕하세요".into(),
            pronunciation: String::new(),
            source_language_code: "cn".into(),
            language_code: "ko".into(),
            direction: TranslationDirection::Forward,
        }
    }

    #[test]
    fn obvious_errors_are_rejected_without_creating_cache() {
        for text in [
            "",
            "[BLANK_AUDIO]",
            "<|nospeech|>",
            "!!!???",
            "啊啊啊啊啊啊啊啊",
        ] {
            assert!(is_obviously_bad(text), "{text}");
        }
        for text in ["你好！", "谢谢您", "Hello, world!", "ㅋㅋㅋㅋ"] {
            assert!(!is_obviously_bad(text), "{text}");
        }
    }

    #[test]
    fn short_time_duplicate_does_not_add_record() {
        let path = test_path();
        let mut cache = VoiceWordCache::load(path.clone()).unwrap();
        let request = || VoiceCacheRequest {
            candidate: candidate("你好"),
            session_id: 1,
            previous: None,
            next: None,
        };
        assert!(cache.enqueue(request()).unwrap());
        assert!(!cache.enqueue(request()).unwrap());
        assert_eq!(cache.pending_len(), 1);
        assert_eq!(VoiceWordCache::load(path.clone()).unwrap().pending_len(), 1);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn review_keeps_uncertain_until_new_context_and_removes_completed() {
        let path = test_path();
        let mut cache = VoiceWordCache::load(path.clone()).unwrap();
        let request = |source| VoiceCacheRequest {
            candidate: candidate(source),
            session_id: 7,
            previous: None,
            next: None,
        };
        cache.enqueue(request("你好")).unwrap();
        let first = cache.review_cases();
        assert_eq!(first.len(), 1);
        cache
            .apply_reviews(first, vec![VoiceVerdict::Uncertain], |_| Ok(()))
            .unwrap();
        assert!(cache.review_cases().is_empty());

        cache.enqueue(request("谢谢您")).unwrap();
        let with_neighbor = cache.review_cases();
        assert_eq!(with_neighbor.len(), 2);
        assert_eq!(with_neighbor[0].1.next.as_deref(), Some("谢谢您"));
        cache
            .apply_reviews(
                with_neighbor,
                vec![VoiceVerdict::Normal, VoiceVerdict::Error],
                |_| Ok(()),
            )
            .unwrap();
        assert_eq!(cache.pending_len(), 0);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn uncertain_entry_expires_only_when_no_new_evidence_arrives() {
        let path = test_path();
        let mut cache = VoiceWordCache::load(path.clone()).unwrap();
        cache
            .enqueue(VoiceCacheRequest {
                candidate: candidate("你好"),
                session_id: 9,
                previous: None,
                next: None,
            })
            .unwrap();
        let selected = cache.review_cases();
        cache
            .apply_reviews(selected, vec![VoiceVerdict::Uncertain], |_| Ok(()))
            .unwrap();
        cache.file.records[0].uncertain_at = Some(now_secs() - RETENTION.as_secs() - 1);
        assert_eq!(cache.expire_uncertain().unwrap(), 1);
        assert_eq!(cache.pending_len(), 0);
        fs::remove_file(path).unwrap();
    }

    fn test_path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "voice_cache_test_{}_{}.db",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
}
