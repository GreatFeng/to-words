//! 可选的语音耗时诊断。默认关闭；启用 `voice-metrics` feature 后追加 JSONL。
//! 即使启用，也不记录音频、识别文字、译文或 API Key。

#[cfg(not(feature = "voice-metrics"))]
use anyhow::Result;
#[cfg(feature = "voice-metrics")]
use anyhow::{Context, Result};
#[cfg(feature = "voice-metrics")]
use serde_json::json;
#[cfg(feature = "voice-metrics")]
use std::collections::HashMap;
#[cfg(feature = "voice-metrics")]
use std::fs::{self, File, OpenOptions};
#[cfg(feature = "voice-metrics")]
use std::io::Write;
#[cfg(feature = "voice-metrics")]
use std::path::Path;
#[cfg(feature = "voice-metrics")]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(feature = "voice-metrics")]
use std::sync::{Arc, Mutex};
#[cfg(not(feature = "voice-metrics"))]
use std::time::Duration;
#[cfg(feature = "voice-metrics")]
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[cfg(feature = "voice-metrics")]
static NEXT_SESSION: AtomicU64 = AtomicU64::new(1);

#[cfg(feature = "voice-metrics")]
#[derive(Clone)]
pub(crate) struct VoiceTrace {
    started: Instant,
    session_id: String,
    inner: Arc<Mutex<TraceFile>>,
}

#[cfg(feature = "voice-metrics")]
struct TraceFile {
    file: File,
    speech_end_ms: HashMap<u64, u64>,
}

#[cfg(feature = "voice-metrics")]
impl VoiceTrace {
    pub(crate) fn new(model_name: &str) -> Result<Self> {
        let directory = crate::project_directory().join("logs");
        fs::create_dir_all(&directory)
            .with_context(|| format!("无法创建语音诊断日志目录：{}", directory.display()))?;
        let path = directory.join("voice_metrics.jsonl");
        Self::open_at(&path, model_name)
    }

    fn open_at(path: &Path, model_name: &str) -> Result<Self> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("无法写入语音诊断日志：{}", path.display()))?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let session_id = format!(
            "{timestamp}-{}-{}",
            std::process::id(),
            NEXT_SESSION.fetch_add(1, Ordering::Relaxed)
        );
        let trace = Self {
            started: Instant::now(),
            session_id,
            inner: Arc::new(Mutex::new(TraceFile {
                file,
                speech_end_ms: HashMap::new(),
            })),
        };
        trace.record(0, "session_started", None, model_name);
        Ok(trace)
    }

    pub(crate) fn elapsed_ms(&self) -> u64 {
        milliseconds(self.started.elapsed())
    }

    pub(crate) fn record(
        &self,
        utterance_id: u64,
        stage: &str,
        duration: Option<Duration>,
        result: &str,
    ) {
        self.write(utterance_id, stage, self.elapsed_ms(), duration, result);
    }

    pub(crate) fn speech_end(&self, utterance_id: u64, trailing_silence: Duration) {
        let elapsed_ms = self
            .elapsed_ms()
            .saturating_sub(milliseconds(trailing_silence));
        if let Ok(mut inner) = self.inner.lock() {
            inner.speech_end_ms.insert(utterance_id, elapsed_ms);
        }
        self.write(utterance_id, "speech_end_estimated", elapsed_ms, None, "ok");
    }

    pub(crate) fn finish(&self, utterance_id: u64, result: &str) {
        let duration_ms = self
            .inner
            .lock()
            .ok()
            .and_then(|mut inner| inner.speech_end_ms.remove(&utterance_id))
            .map(|speech_end_ms| self.elapsed_ms().saturating_sub(speech_end_ms));
        self.write_ms(
            utterance_id,
            "finished",
            self.elapsed_ms(),
            duration_ms,
            result,
        );
    }

    fn write(
        &self,
        utterance_id: u64,
        stage: &str,
        since_session_ms: u64,
        duration: Option<Duration>,
        result: &str,
    ) {
        self.write_ms(
            utterance_id,
            stage,
            since_session_ms,
            duration.map(milliseconds),
            result,
        );
    }

    fn write_ms(
        &self,
        utterance_id: u64,
        stage: &str,
        since_session_ms: u64,
        duration_ms: Option<u64>,
        result: &str,
    ) {
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let entry = json!({
            "schema_version": 1,
            "timestamp_ms": timestamp_ms,
            "session_id": self.session_id,
            "utterance_id": utterance_id,
            "stage": stage,
            "since_session_ms": since_session_ms,
            "duration_ms": duration_ms,
            "result": result,
        });
        if let Ok(mut inner) = self.inner.lock() {
            let _ = serde_json::to_writer(&mut inner.file, &entry);
            let _ = writeln!(inner.file);
        }
    }
}

#[cfg(feature = "voice-metrics")]
fn milliseconds(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(not(feature = "voice-metrics"))]
#[derive(Clone)]
pub(crate) struct VoiceTrace;

#[cfg(not(feature = "voice-metrics"))]
impl VoiceTrace {
    pub(crate) fn new(_model_name: &str) -> Result<Self> {
        Ok(Self)
    }

    pub(crate) fn record(
        &self,
        _utterance_id: u64,
        _stage: &str,
        _duration: Option<Duration>,
        _result: &str,
    ) {
    }

    pub(crate) fn speech_end(&self, _utterance_id: u64, _trailing_silence: Duration) {}

    pub(crate) fn finish(&self, _utterance_id: u64, _result: &str) {}
}

#[cfg(all(test, feature = "voice-metrics"))]
mod tests {
    use super::VoiceTrace;

    #[test]
    fn writes_parseable_content_free_timing_records() {
        let path = std::env::temp_dir().join(format!(
            "to_words_voice_metrics_test_{}_{}.jsonl",
            std::process::id(),
            super::NEXT_SESSION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let trace = VoiceTrace::open_at(&path, "ggml-base.bin").unwrap();
        trace.record(1, "speech_started", None, "ok");
        trace.speech_end(1, std::time::Duration::ZERO);
        trace.finish(1, "dictionary_copied");
        drop(trace);
        let lines = std::fs::read_to_string(&path).unwrap();
        let _ = std::fs::remove_file(path);
        let records: Vec<serde_json::Value> = lines
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(records.len(), 4);
        assert_eq!(records[3]["stage"], "finished");
        assert_eq!(records[3]["result"], "dictionary_copied");
        assert!(records[3]["duration_ms"].is_number());
        assert!(!lines.contains("识别文字"));
    }
}
