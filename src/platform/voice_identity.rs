//! Local owner voice enrollment and speaker verification. Only embeddings are persisted.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use crate::domain::storage;
use crate::platform::voice::{self, SAMPLE_RATE, SpeechGate};
use crate::platform::voice_aec;
use crate::platform::voice_metrics::VoiceTrace;

const SETTING_NAME: &str = "voice_owner_profile_v1";
const MODEL_NAME: &str = "3dspeaker_speech_campplus_sv_zh-cn_16k-common.onnx";
const MATCH_THRESHOLD: f32 = 0.50;
const REQUIRED_SAMPLES: usize = 3;
const SHORT_CLIP_TARGET: usize = SAMPLE_RATE * 6 / 5;

#[derive(Serialize, Deserialize)]
struct OwnerProfile {
    model: String,
    embeddings: Vec<Vec<f32>>,
    #[serde(default)]
    playback_filter: bool,
}

pub(crate) enum EnrollmentEvent {
    Captured(usize),
    Finished(Result<()>),
}

pub(crate) fn model_path() -> std::path::PathBuf {
    crate::project_directory()
        .join("assets")
        .join("voice")
        .join("speaker")
        .join(MODEL_NAME)
}

pub(crate) fn is_registered() -> bool {
    load_profile().ok().flatten().is_some_and(|profile| {
        profile.model == MODEL_NAME && profile.embeddings.len() >= REQUIRED_SAMPLES
    })
}

pub(crate) fn registration_uses_playback_filter() -> Option<bool> {
    load_profile().ok().flatten().and_then(|profile| {
        (profile.model == MODEL_NAME && profile.embeddings.len() >= REQUIRED_SAMPLES)
            .then_some(profile.playback_filter)
    })
}

pub(crate) fn clear_profile() -> Result<()> {
    let connection = storage::open()?;
    connection.execute("DELETE FROM app_settings WHERE name=?1", [SETTING_NAME])?;
    Ok(())
}

fn load_profile() -> Result<Option<OwnerProfile>> {
    storage::load_setting(SETTING_NAME)?
        .map(|value| serde_json::from_str(&value).context("声纹资料已损坏"))
        .transpose()
}

fn require_model() -> Result<std::path::PathBuf> {
    let path = model_path();
    if !path.is_file() {
        bail!("缺少声纹模型：{}", path.display());
    }
    Ok(path)
}

#[cfg(windows)]
pub(crate) struct OwnerVerifier {
    extractor: sherpa_onnx::SpeakerEmbeddingExtractor,
    embeddings: Vec<Vec<f32>>,
}

#[cfg(not(windows))]
pub(crate) struct OwnerVerifier;

#[cfg(not(windows))]
impl OwnerVerifier {
    pub(crate) fn load(_playback_filter: bool) -> Result<Self> {
        bail!("声纹验证仅支持 Windows")
    }
    pub(crate) fn select_owner_audio(&self, _samples: &[i16]) -> Result<Option<Vec<i16>>> {
        bail!("声纹验证仅支持 Windows")
    }
}

#[cfg(windows)]
impl OwnerVerifier {
    pub(crate) fn load(playback_filter: bool) -> Result<Self> {
        let model = require_model()?;
        let profile = load_profile()?.context("尚未注册本人声纹，请先在设置中录制三段语音")?;
        if profile.model != MODEL_NAME || profile.embeddings.len() < REQUIRED_SAMPLES {
            bail!("声纹资料与当前模型不兼容，请重新注册");
        }
        if profile.playback_filter != playback_filter {
            bail!("声纹注册与当前“过滤电脑播放声”设置不一致，请在当前设置下重新注册本人声纹");
        }
        let extractor = create_extractor(&model)?;
        if profile
            .embeddings
            .iter()
            .any(|item| item.len() != extractor.dim() as usize)
        {
            bail!("声纹维度与当前模型不一致，请重新注册");
        }
        Ok(Self {
            extractor,
            embeddings: profile.embeddings,
        })
    }

    pub(crate) fn select_owner_audio(&self, samples: &[i16]) -> Result<Option<Vec<i16>>> {
        let turns = split_turns(samples);
        let mut selected = Vec::new();
        for (start, end) in turns {
            let turn = &samples[start..end];
            let Some((voice_start, voice_end)) = voiced_bounds(turn) else {
                continue;
            };
            let voice = &turn[voice_start..voice_end];
            if voice.len() < SAMPLE_RATE / 10 {
                continue;
            }
            let embedding = compute_embedding(&self.extractor, voice)?;
            if self
                .embeddings
                .iter()
                .any(|owner| cosine(owner, &embedding) >= MATCH_THRESHOLD)
            {
                if !selected.is_empty() {
                    selected.extend(std::iter::repeat_n(0_i16, SAMPLE_RATE / 10));
                }
                selected.extend_from_slice(voice);
            }
        }
        Ok((!selected.is_empty()).then_some(selected))
    }
}

#[cfg(windows)]
fn create_extractor(path: &std::path::Path) -> Result<sherpa_onnx::SpeakerEmbeddingExtractor> {
    let config = sherpa_onnx::SpeakerEmbeddingExtractorConfig {
        model: Some(path.to_string_lossy().into_owned()),
        num_threads: 2,
        debug: false,
        provider: Some("cpu".to_string()),
    };
    sherpa_onnx::SpeakerEmbeddingExtractor::create(&config).context("无法加载说话人声纹模型")
}

#[cfg(windows)]
fn compute_embedding(
    extractor: &sherpa_onnx::SpeakerEmbeddingExtractor,
    samples: &[i16],
) -> Result<Vec<f32>> {
    let stream = extractor.create_stream().context("无法创建声纹分析流")?;
    let mut waveform = samples
        .iter()
        .map(|sample| *sample as f32 / 32768.0)
        .collect::<Vec<_>>();
    if !waveform.is_empty() && waveform.len() < SHORT_CLIP_TARGET {
        let original = waveform.clone();
        while waveform.len() < SHORT_CLIP_TARGET {
            let remaining = SHORT_CLIP_TARGET - waveform.len();
            waveform.extend_from_slice(&original[..remaining.min(original.len())]);
        }
    }
    stream.accept_waveform(SAMPLE_RATE as i32, &waveform);
    stream.input_finished();
    if !extractor.is_ready(&stream) {
        bail!("语音太短，无法提取声纹");
    }
    let embedding = extractor.compute(&stream).context("声纹提取失败")?;
    if embedding.iter().any(|value| !value.is_finite()) {
        bail!("声纹模型返回了无效数据");
    }
    Ok(embedding)
}

fn cosine(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() || left.is_empty() {
        return -1.0;
    }
    let dot = left.iter().zip(right).map(|(a, b)| a * b).sum::<f32>();
    let ln = left.iter().map(|x| x * x).sum::<f32>().sqrt();
    let rn = right.iter().map(|x| x * x).sum::<f32>().sqrt();
    if ln <= f32::EPSILON || rn <= f32::EPSILON {
        -1.0
    } else {
        dot / (ln * rn)
    }
}

/// Trim long leading/trailing silence so a short spoken word is not represented
/// mostly by the VAD gate's surrounding quiet audio during speaker verification.
fn voiced_bounds(samples: &[i16]) -> Option<(usize, usize)> {
    let frame_size = SAMPLE_RATE / 100;
    let levels = samples
        .chunks(frame_size)
        .map(|frame| {
            (frame
                .iter()
                .map(|sample| (*sample as f64).powi(2))
                .sum::<f64>()
                / frame.len().max(1) as f64)
                .sqrt() as f32
        })
        .collect::<Vec<_>>();
    let peak = levels.iter().copied().fold(0.0_f32, f32::max);
    if peak < 80.0 {
        return None;
    }
    let threshold = (peak * 0.08).max(80.0);
    let first = levels.iter().position(|level| *level >= threshold)?;
    let last = levels.iter().rposition(|level| *level >= threshold)?;
    let margin = SAMPLE_RATE / 20;
    Some((
        (first * frame_size).saturating_sub(margin),
        ((last + 1) * frame_size + margin).min(samples.len()),
    ))
}

/// Find turns separated by a short, clearly quiet gap inside a VAD utterance.
/// This can separate alternating speakers, but not overlapping voices.
fn split_turns(samples: &[i16]) -> Vec<(usize, usize)> {
    const FRAME: usize = SAMPLE_RATE / 100;
    const GAP_FRAMES: usize = 18;
    let rms = samples
        .chunks(FRAME)
        .map(|frame| {
            (frame
                .iter()
                .map(|sample| (*sample as f64).powi(2))
                .sum::<f64>()
                / frame.len().max(1) as f64)
                .sqrt() as f32
        })
        .collect::<Vec<_>>();
    let peak = rms.iter().copied().fold(0.0_f32, f32::max);
    let threshold = (peak * 0.15).max(80.0);
    let mut turns = Vec::new();
    let mut start = 0;
    let mut quiet_start = None;
    for (index, level) in rms.iter().enumerate() {
        if *level < threshold {
            quiet_start.get_or_insert(index);
        } else if let Some(first_quiet) = quiet_start.take() {
            if index - first_quiet >= GAP_FRAMES {
                let split = ((first_quiet + index) / 2 * FRAME).min(samples.len());
                if split.saturating_sub(start) >= SAMPLE_RATE / 10 {
                    turns.push((start, split));
                    start = split;
                }
            }
        }
    }
    if samples.len().saturating_sub(start) >= SAMPLE_RATE / 10 {
        turns.push((start, samples.len()));
    }
    if turns.is_empty() {
        turns.push((0, samples.len()));
    }
    turns
}

#[cfg(windows)]
pub(crate) fn start_enrollment(playback_filter: bool) -> Result<Receiver<EnrollmentEvent>> {
    let model = require_model()?;
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let result = enroll(model, playback_filter, &sender);
        let _ = sender.send(EnrollmentEvent::Finished(result));
    });
    Ok(receiver)
}

#[cfg(windows)]
fn enroll(
    model: std::path::PathBuf,
    playback_filter: bool,
    events: &mpsc::Sender<EnrollmentEvent>,
) -> Result<()> {
    let trace = VoiceTrace::new("speaker_enrollment")?;
    let stop = Arc::new(AtomicBool::new(false));
    let worker_stop = Arc::clone(&stop);
    let (audio_tx, audio_rx) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        if playback_filter {
            voice_aec::record(worker_stop, audio_tx, true, trace)
        } else {
            voice::record_microphone(worker_stop, audio_tx, true, trace)
        }
    });
    let mut gate = SpeechGate::new(0.7, true);
    let mut segments = Vec::with_capacity(REQUIRED_SAMPLES);
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(40) && segments.len() < REQUIRED_SAMPLES {
        match audio_rx.recv_timeout(Duration::from_millis(150)) {
            Ok(chunk) => {
                if let Some(segment) = gate.push(&chunk).completed {
                    if segment.samples.len() >= SAMPLE_RATE * 3 / 2 {
                        segments.push(segment.samples);
                        let _ = events.send(EnrollmentEvent::Captured(segments.len()));
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) if worker.is_finished() => break,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    stop.store(true, Ordering::Release);
    worker
        .join()
        .map_err(|_| anyhow::anyhow!("声纹录音线程意外退出"))??;
    if segments.len() < REQUIRED_SAMPLES {
        bail!("声纹注册需要三段各至少 1.5 秒的语音，请每段结束后停顿约一秒再说下一段");
    }
    let extractor = create_extractor(&model)?;
    let embeddings = segments
        .iter()
        .map(|segment| {
            let (start, end) = voiced_bounds(segment).context("注册语音中没有检测到清晰的人声")?;
            compute_embedding(&extractor, &segment[start..end])
        })
        .collect::<Result<Vec<_>>>()?;
    if embeddings.iter().enumerate().any(|(index, left)| {
        embeddings
            .iter()
            .skip(index + 1)
            .any(|right| cosine(left, right) < MATCH_THRESHOLD)
    }) {
        bail!("三段注册语音差异过大，请确认使用同一麦克风并在实际使用环境中重新录制");
    }
    let profile = OwnerProfile {
        model: MODEL_NAME.to_string(),
        embeddings,
        playback_filter,
    };
    storage::save_setting(SETTING_NAME, &serde_json::to_string(&profile)?)?;
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn start_enrollment(_playback_filter: bool) -> Result<Receiver<EnrollmentEvent>> {
    bail!("声纹注册仅支持 Windows")
}

#[cfg(test)]
mod tests {
    use super::{SAMPLE_RATE, cosine, split_turns, voiced_bounds};

    #[test]
    fn old_voiceprints_default_to_unfiltered_capture() {
        let profile: super::OwnerProfile =
            serde_json::from_str(r#"{"model":"test","embeddings":[[0.1,0.2]]}"#).unwrap();
        assert!(!profile.playback_filter);
    }

    #[test]
    fn cosine_rejects_invalid_shapes() {
        assert_eq!(cosine(&[], &[]), -1.0);
        assert_eq!(cosine(&[1.0], &[1.0, 2.0]), -1.0);
        assert!((cosine(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-5);
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]) < 0.1);
    }

    #[test]
    fn quiet_gap_separates_alternating_speakers() {
        let mut samples = vec![2_000_i16; SAMPLE_RATE];
        samples.extend(vec![0_i16; SAMPLE_RATE / 4]);
        samples.extend(vec![1_500_i16; SAMPLE_RATE]);
        let turns = split_turns(&samples);
        assert_eq!(turns.len(), 2);
        assert!(turns[0].1 <= turns[1].0);
    }

    #[test]
    fn short_word_remains_one_turn() {
        assert_eq!(
            split_turns(&vec![1_000_i16; SAMPLE_RATE / 2]),
            vec![(0, SAMPLE_RATE / 2)]
        );
    }

    #[test]
    fn short_word_is_trimmed_without_losing_speech() {
        let mut samples = vec![0_i16; SAMPLE_RATE / 3];
        samples.extend(vec![1_000_i16; SAMPLE_RATE / 5]);
        samples.extend(vec![0_i16; SAMPLE_RATE / 2]);
        let (start, end) = voiced_bounds(&samples).unwrap();
        assert!(start < SAMPLE_RATE / 3);
        assert!(end > SAMPLE_RATE / 3 + SAMPLE_RATE / 5);
        assert!(end - start < SAMPLE_RATE / 2);
        assert_eq!(voiced_bounds(&vec![0_i16; SAMPLE_RATE]), None);
    }
}
