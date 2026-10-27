//! 本地语音识别：Windows 麦克风录音，默认使用 SenseVoice，也支持 Whisper 和实验版 Windows AI。
//!
//! 录音始终在后台线程进行。连续两帧检测到语音后开始收集句子，
//! 约 0.6 秒静音后自动提交；保持输入时继续收集下一句。

use crate::i18n::{self, tr};
use crate::platform::voice_metrics::VoiceTrace;
use crate::platform::{
    streaming_asr, voice_aec, voice_identity, voice_path_compat, voice_software_aec,
};
use anyhow::{Context, Result, bail};
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

pub(crate) const SAMPLE_RATE: usize = 16_000;
const MAX_SECONDS: usize = 90;
const MAX_UTTERANCE_SECONDS: usize = 30;
const CALIBRATION_FRAMES: usize = 4;
const SHORT_PAUSE_GRACE: Duration = Duration::from_millis(450);
const SHORT_SEGMENT_SAMPLES: usize = SAMPLE_RATE * 12 / 10;
static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) enum VoiceEvent {
    SpeechStarted(u64),
    Partial(u64, String),
    Recognizing(u64),
    Utterance(u64, Result<String, String>),
    Filtered(u64),
    CaptureReleased,
    Stopped(Option<String>),
}

enum StreamCommand {
    Start(u64, Vec<i16>),
    Audio(u64, Vec<i16>),
    End,
}

fn start_streaming_subtitles(sender: Sender<VoiceEvent>) -> Option<Sender<StreamCommand>> {
    let root =
        crate::web_config::project_directory().join("assets/voice/streaming/zipformer-small-zh-en");
    if ![
        "encoder-epoch-99-avg-1.int8.onnx",
        "decoder-epoch-99-avg-1.onnx",
        "joiner-epoch-99-avg-1.int8.onnx",
        "tokens.txt",
    ]
    .iter()
    .all(|name| root.join(name).is_file())
    {
        return None;
    }
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut model = match streaming_asr::StreamingAsr::load() {
            Ok(Some(model)) => model,
            Ok(None) => return,
            Err(reason) => {
                eprintln!("流式字幕模型加载失败：{reason:#}");
                return;
            }
        };
        let mut active_id = None;
        for command in rx {
            let (id, samples) = match command {
                StreamCommand::Start(id, samples) => {
                    model.reset();
                    active_id = Some(id);
                    (id, samples)
                }
                StreamCommand::Audio(id, samples) if active_id == Some(id) => (id, samples),
                StreamCommand::End => {
                    model.reset();
                    active_id = None;
                    continue;
                }
                _ => continue,
            };
            if let Some(text) = model.accept(&samples) {
                let _ = sender.send(VoiceEvent::Partial(id, text));
            }
        }
    });
    Some(tx)
}

pub(crate) struct VoiceSession {
    stop: Arc<AtomicBool>,
    discard_on_stop: Arc<AtomicBool>,
    pub(crate) trace: VoiceTrace,
    pub(crate) receiver: Receiver<VoiceEvent>,
    pub(crate) stopping: bool,
}

enum RecognitionEngine {
    SenseVoice(PathBuf, PathBuf, PathBuf),
    Whisper(PathBuf, PathBuf),
}

impl VoiceSession {
    pub(crate) fn stop(&mut self) {
        self.stopping = true;
        self.stop.store(true, Ordering::Release);
    }

    /// Switch recording modes without transcribing the abandoned tail. The
    /// caller may start a new capture after CaptureReleased, while old ASR
    /// cleanup completes independently.
    pub(crate) fn cancel_for_handoff(&mut self) {
        self.discard_on_stop.store(true, Ordering::Release);
        self.stop();
    }
}

pub(crate) fn engine_status(backend: &str) -> String {
    let status = match selected_engine(backend) {
        Ok(RecognitionEngine::SenseVoice(_, model, _)) => i18n::message(
            "SenseVoice 模型：{model}",
            &[(
                "model",
                model
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(tr("未知模型")),
            )],
        ),
        Ok(RecognitionEngine::Whisper(_, model)) => i18n::message(
            "whisper.cpp 模型：{model}",
            &[(
                "model",
                model
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(tr("未知模型")),
            )],
        ),
        Err(error) => {
            let detail = error.to_string();
            let detail = if let Some(path) = detail.strip_prefix("缺少 SenseVoice 文件：") {
                i18n::message("缺少 SenseVoice 文件：{path}", &[("path", path)])
            } else if let Some(path) =
                detail.strip_prefix("缺少多语言模型，请将 ggml-base.bin 放在 ")
            {
                i18n::message(
                    "缺少多语言模型，请将 ggml-base.bin 放在 {path}",
                    &[("path", path)],
                )
            } else if let Some(path) = detail.strip_prefix("缺少 ") {
                i18n::message("缺少文件：{path}", &[("path", path)])
            } else {
                detail
            };
            i18n::message("语音识别尚未就绪：{error}", &[("error", &detail)])
        }
    };
    let notice = voice_path_compat::path_warning().or_else(|| match selected_engine(backend) {
        Ok(RecognitionEngine::SenseVoice(executable, model, vad)) => {
            voice_path_compat::resource_notice(&executable, &model, Some(&vad))
        }
        Ok(RecognitionEngine::Whisper(executable, model)) => {
            voice_path_compat::resource_notice(&executable, &model, None)
        }
        Err(_) => None,
    });
    if let Some(notice) = notice {
        format!("{status}\n{notice}")
    } else {
        status
    }
}

pub(crate) fn start(
    keep_input: bool,
    push_to_talk: bool,
    language: &str,
    backend: &str,
    strict_language: Option<&str>,
    silence_seconds: f32,
    owner_filter: bool,
    playback_filter: bool,
) -> Result<VoiceSession> {
    start_with_idle_timeout(
        keep_input,
        push_to_talk,
        language,
        backend,
        strict_language,
        silence_seconds,
        owner_filter,
        playback_filter,
        Duration::from_secs(15),
    )
}

pub(crate) fn start_with_idle_timeout(
    keep_input: bool,
    push_to_talk: bool,
    language: &str,
    backend: &str,
    strict_language: Option<&str>,
    silence_seconds: f32,
    owner_filter: bool,
    playback_filter: bool,
    no_speech_timeout: Duration,
) -> Result<VoiceSession> {
    start_with_source(
        keep_input,
        push_to_talk,
        language,
        backend,
        strict_language,
        silence_seconds,
        owner_filter,
        playback_filter,
        false,
        None,
        no_speech_timeout,
    )
}

pub(crate) fn start_playback(
    backend: &str,
    silence_seconds: f32,
    playback_device: &str,
    language: &str,
) -> Result<VoiceSession> {
    start_with_source(
        true,
        false,
        language,
        backend,
        None,
        silence_seconds,
        false,
        false,
        true,
        Some(playback_device),
        Duration::from_secs(15),
    )
}

fn start_with_source(
    keep_input: bool,
    push_to_talk: bool,
    language: &str,
    backend: &str,
    strict_language: Option<&str>,
    silence_seconds: f32,
    owner_filter: bool,
    playback_filter: bool,
    system_playback: bool,
    playback_device: Option<&str>,
    no_speech_timeout: Duration,
) -> Result<VoiceSession> {
    let engine = selected_engine(backend)?;
    let engine = match engine {
        RecognitionEngine::SenseVoice(executable, model, vad)
            if system_playback && sensevoice_language_token(language).is_some() =>
        {
            let model = prepare_subtitle_language_model(&model, language)?;
            RecognitionEngine::SenseVoice(executable, model, vad)
        }
        other => other,
    };
    let background = keep_input && !push_to_talk;
    let owner_verifier = if owner_filter {
        Some(voice_identity::OwnerVerifier::load(playback_filter)?)
    } else {
        None
    };
    let language = whisper_language(language).to_string();
    let strict_language = strict_language.map(str::to_owned);
    let playback_device = playback_device.unwrap_or_default().to_owned();
    let trace_name = match &engine {
        RecognitionEngine::SenseVoice(_, model, _) => model
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("sensevoice"),
        RecognitionEngine::Whisper(_, model) => model
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("unknown_model"),
    };
    let trace = VoiceTrace::new(trace_name)?;
    trace.record(
        0,
        "keep_input",
        None,
        if keep_input { "enabled" } else { "disabled" },
    );
    if matches!(engine, RecognitionEngine::Whisper(_, _)) {
        trace.record(
            0,
            "asr_threads",
            None,
            &whisper_thread_count(background).to_string(),
        );
    }
    trace.record(
        0,
        "asr_process_priority",
        None,
        if background { "below_normal" } else { "normal" },
    );
    #[cfg(not(windows))]
    bail!("语音识别目前仅支持 Windows");

    let stop = Arc::new(AtomicBool::new(false));
    let discard_on_stop = Arc::new(AtomicBool::new(false));
    let (sender, receiver) = mpsc::channel();
    let worker_stop = Arc::clone(&stop);
    let worker_discard = Arc::clone(&discard_on_stop);
    let worker_trace = trace.clone();
    std::thread::spawn(move || {
        let (audio_sender, audio_receiver) = mpsc::channel();
        let streaming = if system_playback && matches!(language.as_str(), "zh" | "en") {
            start_streaming_subtitles(sender.clone())
        } else {
            None
        };
        let (segment_sender, segment_receiver) = mpsc::channel::<(u64, Vec<i16>)>();
        let asr_sender = sender.clone();
        let asr_trace = worker_trace.clone();
        let asr_stop = Arc::clone(&worker_stop);
        let asr_discard = Arc::clone(&worker_discard);
        let transcriber = std::thread::spawn(move || {
            for (utterance_id, mut samples) in segment_receiver {
                if asr_discard.load(Ordering::Acquire) {
                    continue;
                }
                if let Some(verifier) = &owner_verifier {
                    match verifier.select_owner_audio(&samples) {
                        Ok(Some(owner_audio)) => {
                            samples = owner_audio;
                            asr_trace.record(utterance_id, "speaker_check", None, "accepted");
                        }
                        Ok(None) => {
                            asr_trace.record(utterance_id, "speaker_check", None, "rejected");
                            let _ = asr_sender.send(VoiceEvent::Filtered(utterance_id));
                            continue;
                        }
                        Err(error) => {
                            let _ = asr_sender.send(VoiceEvent::Utterance(
                                utterance_id,
                                Err(format!("声纹验证失败：{error:#}")),
                            ));
                            continue;
                        }
                    }
                }
                let asr_started = Instant::now();
                asr_trace.record(utterance_id, "asr_started", None, "ok");
                let result =
                    transcribe_selected(&engine, &samples, &language, background, system_playback)
                        .and_then(|text| {
                            if let Some(expected) = strict_language.as_deref()
                                && !crate::translation_language::accepts_same_language_text(
                                    &text, expected,
                                )
                            {
                                bail!(
                                    "识别结果不是所选的{}，已忽略",
                                    crate::translation_language::language_label(expected)
                                );
                            }
                            Ok(text)
                        })
                        .map_err(|error| format!("{error:#}"));
                asr_trace.record(
                    utterance_id,
                    "asr_completed",
                    Some(asr_started.elapsed()),
                    match &result {
                        Ok(text) if text.trim().is_empty() => "empty",
                        Ok(_) => "ok",
                        Err(_) => "error",
                    },
                );
                if push_to_talk {
                    while !asr_stop.load(Ordering::Acquire) {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                }
                let _ = asr_sender.send(VoiceEvent::Utterance(utterance_id, result));
            }
        });
        let recorder_stop = Arc::clone(&worker_stop);
        let recorder_trace = worker_trace.clone();
        let recorder = std::thread::spawn(move || {
            if system_playback {
                voice_software_aec::record_playback(recorder_stop, audio_sender, &playback_device)
            } else if playback_filter {
                voice_aec::record(recorder_stop, audio_sender, keep_input, recorder_trace)
            } else {
                record_microphone(recorder_stop, audio_sender, keep_input, recorder_trace)
            }
        });
        let mut gate = if system_playback {
            SpeechGate::new_playback(silence_seconds)
        } else {
            SpeechGate::new(silence_seconds, !push_to_talk)
        };
        let mut auto_stopped = false;
        let mut heard_speech = false;
        let mut utterance_id = 0;
        let mut pending_short: Option<(CompletedSegment, Instant)> = None;
        let started_at = Instant::now();
        loop {
            if !keep_input && !heard_speech && started_at.elapsed() >= no_speech_timeout {
                worker_stop.store(true, Ordering::Release);
                worker_trace.record(0, "no_speech_timeout", None, "timeout");
                break;
            }
            match audio_receiver.recv_timeout(Duration::from_millis(50)) {
                Ok(chunk) => {
                    let update = gate.push(&chunk);
                    if update.started {
                        heard_speech = true;
                        if let Some((previous, _)) = pending_short.take() {
                            gate.resume_short_segment(previous);
                            worker_trace.record(utterance_id, "short_pause_merged", None, "ok");
                        } else {
                            utterance_id += 1;
                            worker_trace.record(utterance_id, "speech_started", None, "ok");
                            let _ = sender.send(VoiceEvent::SpeechStarted(utterance_id));
                            if let Some(tx) = &streaming {
                                let _ = tx
                                    .send(StreamCommand::Start(utterance_id, gate.segment.clone()));
                            }
                        }
                    } else if let Some(tx) = &streaming
                        && (gate.active || update.completed.is_some())
                    {
                        let _ = tx.send(StreamCommand::Audio(utterance_id, chunk));
                    }
                    if let Some(segment) = update.completed {
                        if let Some(tx) = &streaming {
                            let _ = tx.send(StreamCommand::End);
                        }
                        if !system_playback
                            && segment.reason == "silence"
                            && segment.samples.len() <= SHORT_SEGMENT_SAMPLES
                        {
                            pending_short = Some((segment, Instant::now()));
                            worker_trace.record(utterance_id, "short_pause_wait", None, "ok");
                            continue;
                        }
                        submit_segment(
                            utterance_id,
                            segment,
                            &sender,
                            &segment_sender,
                            &worker_trace,
                        );
                        if !keep_input {
                            worker_stop.store(true, Ordering::Release);
                            auto_stopped = true;
                        }
                        if auto_stopped {
                            break;
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) if recorder.is_finished() => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if pending_short.as_ref().is_some_and(|(_, since)| {
                since.elapsed() >= SHORT_PAUSE_GRACE && gate.candidate_frames == 0
            }) {
                let (segment, _) = pending_short.take().unwrap();
                submit_segment(
                    utterance_id,
                    segment,
                    &sender,
                    &segment_sender,
                    &worker_trace,
                );
                if !keep_input {
                    worker_stop.store(true, Ordering::Release);
                    auto_stopped = true;
                    break;
                }
            }
        }
        worker_stop.store(true, Ordering::Release);
        let recorded = recorder
            .join()
            .map_err(|_| "音频采集线程意外终止".to_string())
            .and_then(|result| result.map_err(|error| format!("{error:#}")));
        let discarded = worker_discard.load(Ordering::Acquire);
        if discarded {
            let _ = sender.send(VoiceEvent::CaptureReleased);
        }
        // 手动停止时还没等到静音，仍提交最后一句；自动停止已经提交过，不再重复识别。
        let min_final_samples = if push_to_talk {
            SAMPLE_RATE / 5
        } else {
            SAMPLE_RATE / 2
        };
        if !discarded
            && !auto_stopped
            && let Some(segment) = gate.take_active(min_final_samples)
        {
            worker_trace.speech_end(utterance_id, Duration::ZERO);
            worker_trace.record(utterance_id, "manual_segment", None, "ok");
            worker_trace.record(
                utterance_id,
                "audio_segment",
                Some(audio_duration(segment.len())),
                "ok",
            );
            let _ = sender.send(VoiceEvent::Recognizing(utterance_id));
            if segment_sender.send((utterance_id, segment)).is_err() {
                let _ = sender.send(VoiceEvent::Utterance(
                    utterance_id,
                    Err("语音识别线程已停止".to_string()),
                ));
            }
        }
        if !discarded && let Some((segment, _)) = pending_short.take() {
            submit_segment(
                utterance_id,
                segment,
                &sender,
                &segment_sender,
                &worker_trace,
            );
        }
        drop(segment_sender);
        let recognized = transcriber
            .join()
            .map_err(|_| "语音识别线程意外终止".to_string());
        let completed = recorded.and(recognized);
        worker_trace.record(
            0,
            "session_stopped",
            None,
            if completed.is_ok() { "ok" } else { "error" },
        );
        let _ = sender.send(VoiceEvent::Stopped(completed.err()));
    });
    Ok(VoiceSession {
        stop,
        discard_on_stop,
        trace,
        receiver,
        stopping: false,
    })
}

fn submit_segment(
    utterance_id: u64,
    segment: CompletedSegment,
    sender: &Sender<VoiceEvent>,
    segment_sender: &Sender<(u64, Vec<i16>)>,
    trace: &VoiceTrace,
) {
    if segment.reason == "silence" {
        trace.speech_end(utterance_id, segment.trailing_silence);
    }
    trace.record(
        utterance_id,
        "vad_complete",
        Some(segment.trailing_silence),
        segment.reason,
    );
    trace.record(
        utterance_id,
        "audio_segment",
        Some(audio_duration(segment.samples.len())),
        "ok",
    );
    let _ = sender.send(VoiceEvent::Recognizing(utterance_id));
    if segment_sender
        .send((utterance_id, segment.samples))
        .is_err()
    {
        let _ = sender.send(VoiceEvent::Utterance(
            utterance_id,
            Err("语音识别线程已停止".to_string()),
        ));
    }
}

/// In same-language mode, use the source language for Whisper and the output
/// filter. SenseVoice still detects the spoken language automatically.
pub(crate) fn effective_voice_language<'a>(
    configured: &'a str,
    source: &'a str,
    target: &str,
) -> &'a str {
    crate::translation_language::same_language_pair(source, target).unwrap_or(configured)
}

/// Language selection must not silently replace the user's selected engine.
pub(crate) fn effective_voice_backend<'a>(
    configured: &'a str,
    _source: &str,
    _target: &str,
) -> &'a str {
    configured
}

pub(crate) struct SpeechGate {
    noise_floor: f32,
    calibration: [f32; CALIBRATION_FRAMES],
    calibrated_frames: usize,
    speech_peak: f32,
    candidate_frames: u8,
    active: bool,
    silent_samples: usize,
    silence_threshold_samples: usize,
    submit_on_silence: bool,
    max_segment_samples: usize,
    playback_start_threshold: Option<f32>,
    preroll: Vec<i16>,
    segment: Vec<i16>,
}

pub(crate) struct GateUpdate {
    started: bool,
    pub(crate) completed: Option<CompletedSegment>,
}

pub(crate) struct CompletedSegment {
    pub(crate) samples: Vec<i16>,
    trailing_silence: Duration,
    reason: &'static str,
}

impl SpeechGate {
    fn resume_short_segment(&mut self, previous: CompletedSegment) {
        let mut combined = previous.samples;
        combined.extend_from_slice(&self.segment);
        self.segment = combined;
    }

    pub(crate) fn new(silence_seconds: f32, submit_on_silence: bool) -> Self {
        let silence_seconds = if silence_seconds.is_finite() {
            silence_seconds.clamp(0.2, 3.0)
        } else {
            0.6
        };
        Self {
            noise_floor: 0.0,
            calibration: [0.0; CALIBRATION_FRAMES],
            calibrated_frames: 0,
            speech_peak: 0.0,
            candidate_frames: 0,
            active: false,
            silent_samples: 0,
            silence_threshold_samples: (silence_seconds * SAMPLE_RATE as f32).round() as usize,
            submit_on_silence,
            max_segment_samples: SAMPLE_RATE * MAX_UTTERANCE_SECONDS,
            playback_start_threshold: None,
            preroll: Vec::new(),
            segment: Vec::new(),
        }
    }

    /// Playback may already contain speech when monitoring starts and videos often
    /// have no full pause. Do not calibrate against that speech; submit bounded
    /// chunks so subtitles continue to appear during uninterrupted playback.
    pub(crate) fn new_playback(silence_seconds: f32) -> Self {
        let mut gate = Self::new(silence_seconds, true);
        gate.calibrated_frames = CALIBRATION_FRAMES;
        // Temporary text stays visible while waiting for a pause. Bound
        // continuous playback so final SenseVoice jobs remain manageable.
        gate.max_segment_samples = SAMPLE_RATE * 5;
        gate.playback_start_threshold = Some(120.0);
        gate
    }

    pub(crate) fn push(&mut self, chunk: &[i16]) -> GateUpdate {
        let mean_square = chunk
            .iter()
            .map(|sample| (*sample as f64).powi(2))
            .sum::<f64>()
            / chunk.len().max(1) as f64;
        let rms = mean_square.sqrt() as f32;
        if !self.active {
            self.preroll.extend_from_slice(chunk);
            let excess = self.preroll.len().saturating_sub(SAMPLE_RATE * 4 / 10);
            self.preroll.drain(..excess);
            if self.calibrated_frames < CALIBRATION_FRAMES {
                self.calibration[self.calibrated_frames] = rms;
                self.calibrated_frames += 1;
                if self.calibrated_frames == CALIBRATION_FRAMES {
                    self.calibration.sort_by(f32::total_cmp);
                    self.noise_floor = self.calibration[1];
                }
                return GateUpdate {
                    started: false,
                    completed: None,
                };
            }
            let threshold = self.playback_start_threshold.unwrap_or_else(|| {
                (self.noise_floor * 1.35)
                    .max(self.noise_floor + 120.0)
                    .max(240.0)
            });
            if rms >= threshold {
                self.candidate_frames = self.candidate_frames.saturating_add(1);
            } else {
                self.candidate_frames = 0;
                if self.playback_start_threshold.is_none() {
                    self.noise_floor = self.noise_floor * 0.95 + rms * 0.05;
                }
            }
            if self.candidate_frames >= 2 {
                self.active = true;
                self.speech_peak = rms;
                self.candidate_frames = 0;
                self.segment = std::mem::take(&mut self.preroll);
                return GateUpdate {
                    started: true,
                    completed: None,
                };
            }
            return GateUpdate {
                started: false,
                completed: None,
            };
        }
        self.segment.extend_from_slice(chunk);
        self.speech_peak = rms.max(self.speech_peak * 0.99);
        let silence_threshold = if self.playback_start_threshold.is_some() {
            (self.speech_peak * 0.18).max(100.0)
        } else {
            // A single loud syllable must not make the quieter remainder of a
            // sentence look like silence. Keep the ambient-noise threshold,
            // but limit how far one speech peak can raise it.
            (self.noise_floor * 1.2)
                .max((self.speech_peak * 0.4).min(900.0))
                .max(240.0)
        };
        if rms < silence_threshold {
            self.silent_samples += chunk.len();
        } else {
            self.silent_samples = 0;
        }
        if (self.submit_on_silence && self.silent_samples >= self.silence_threshold_samples)
            || self.segment.len() >= self.max_segment_samples
        {
            let ended_by_silence =
                self.submit_on_silence && self.silent_samples >= self.silence_threshold_samples;
            let trailing_silence =
                Duration::from_millis((self.silent_samples as u64 * 1000) / SAMPLE_RATE as u64);
            // 留一小段尾部静音，避免截断最后的字音。
            let remove = self.silent_samples.saturating_sub(SAMPLE_RATE / 10);
            self.segment
                .truncate(self.segment.len().saturating_sub(remove));
            self.active = false;
            self.silent_samples = 0;
            self.speech_peak = 0.0;
            let completed = CompletedSegment {
                samples: std::mem::take(&mut self.segment),
                trailing_silence,
                reason: if ended_by_silence {
                    "silence"
                } else {
                    "max_duration"
                },
            };
            return GateUpdate {
                started: false,
                completed: Some(completed),
            };
        }
        GateUpdate {
            started: false,
            completed: None,
        }
    }

    fn take_active(&mut self, min_samples: usize) -> Option<Vec<i16>> {
        if self.active && self.segment.len() >= min_samples {
            Some(std::mem::take(&mut self.segment))
        } else {
            None
        }
    }
}

fn selected_engine(backend: &str) -> Result<RecognitionEngine> {
    match backend {
        "sensevoice" => {
            let (engine, model, vad) = sensevoice_files()?;
            Ok(RecognitionEngine::SenseVoice(engine, model, vad))
        }
        "whisper" => {
            let (engine, model) = engine_files()?;
            Ok(RecognitionEngine::Whisper(engine, model))
        }
        _ => bail!("未知的语音识别引擎：{backend}"),
    }
}

fn transcribe_selected(
    engine: &RecognitionEngine,
    samples: &[i16],
    language: &str,
    background: bool,
    system_playback: bool,
) -> Result<String> {
    match engine {
        RecognitionEngine::SenseVoice(executable, model, vad) => transcribe_sensevoice(
            executable,
            model,
            vad,
            samples,
            background,
            !system_playback,
        ),
        RecognitionEngine::Whisper(executable, model) => {
            transcribe(executable, model, samples, language, background)
        }
    }
}

fn sensevoice_files() -> Result<(PathBuf, PathBuf, PathBuf)> {
    let base = crate::project_directory()
        .join("assets")
        .join("voice")
        .join("funasr");
    sensevoice_files_in(&base)
}

fn sensevoice_files_in(base: &Path) -> Result<(PathBuf, PathBuf, PathBuf)> {
    let executable = base.join("llama-funasr-sensevoice.exe");
    let model = base.join("sensevoice-small-q8.gguf");
    let vad = base.join("fsmn-vad.gguf");
    for path in [&executable, &model, &vad] {
        if !path.is_file() {
            bail!("缺少 SenseVoice 文件：{}", path.display());
        }
    }
    Ok((executable, model, vad))
}

fn sensevoice_language_token(language: &str) -> Option<u32> {
    match language {
        "cn" | "zh" => Some(3),
        "en" => Some(4),
        "ja" => Some(11),
        "ko" => Some(12),
        _ => None,
    }
}

// The bundled GGUF CLI has no language flag. SenseVoice stores its four query
// tokens in GGUF metadata, so a language-specific copy can change just the
// first token without changing the original model used by microphone input.
fn query_language_token(file: &mut fs::File) -> Result<(u64, u32)> {
    let mut header = [0_u8; 4096];
    file.read_exact(&mut header)?;
    let header = &header[..];
    anyhow::ensure!(header.starts_with(b"GGUF"), "SenseVoice 模型不是 GGUF 格式");
    let name = b"sv.query_tokens";
    let start = header
        .windows(name.len())
        .position(|window| window == name)
        .context("SenseVoice 模型缺少语言查询配置")?
        + name.len();
    let metadata = header
        .get(start..start + 32)
        .context("SenseVoice 语言查询配置不完整")?;
    let number = |at: usize| u32::from_le_bytes(metadata[at..at + 4].try_into().unwrap());
    anyhow::ensure!(
        number(0) == 9
            && number(4) == 5
            && u64::from_le_bytes(metadata[8..16].try_into().unwrap()) == 4
            && number(20) == 1
            && number(24) == 2,
        "SenseVoice 模型语言查询格式不受支持"
    );
    Ok(((start + 16) as u64, number(16)))
}

fn prepare_subtitle_language_model(source: &Path, language: &str) -> Result<PathBuf> {
    if let Some(app_data) = std::env::var_os("LOCALAPPDATA") {
        let cache = PathBuf::from(app_data).join("to_words/sensevoice_subtitles");
        if let Ok(model) = prepare_subtitle_language_model_in(source, language, &cache) {
            return Ok(model);
        }
    }
    // Portable installs and restricted Windows accounts can use the model folder.
    let cache = crate::web_config::project_directory().join("assets/voice/funasr/.subtitle_cache");
    prepare_subtitle_language_model_in(source, language, &cache)
}

fn prepare_subtitle_language_model_in(
    source: &Path,
    language: &str,
    cache: &Path,
) -> Result<PathBuf> {
    let token = sensevoice_language_token(language).context("不支持的字幕识别语言")?;
    let source_meta = fs::metadata(source).context("无法读取 SenseVoice 模型信息")?;
    let modified = source_meta
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    fs::create_dir_all(&cache).context("无法创建字幕模型缓存目录")?;
    let target = cache.join(format!(
        "sensevoice-subtitle-{language}-{}-{modified}.gguf",
        source_meta.len()
    ));
    if target.is_file() && fs::metadata(&target)?.len() == source_meta.len() {
        let mut cached = fs::File::open(&target).context("无法读取已缓存的字幕模型")?;
        if query_language_token(&mut cached).is_ok_and(|(_, current)| current == token) {
            return Ok(target);
        }
    }
    let result = (|| {
        fs::copy(source, &target).context("无法缓存字幕识别模型")?;
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&target)
            .context("无法修改字幕模型的语言提示")?;
        let (offset, original) = query_language_token(&mut file)?;
        anyhow::ensure!(original == 0, "SenseVoice 原模型语言并非自动检测");
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(&token.to_le_bytes())?;
        file.flush()?;
        drop(file);
        Ok(target.clone())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&target);
    }
    result
}

fn transcribe_sensevoice(
    executable: &Path,
    model: &Path,
    vad: &Path,
    samples: &[i16],
    background: bool,
    use_vad: bool,
) -> Result<String> {
    let paths = voice_path_compat::prepare(executable, model, Some(vad))?;
    let temp_id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let wav = paths.scratch.join(format!(
        "to_words_sensevoice_{}_{}.wav",
        std::process::id(),
        temp_id
    ));
    let result = (|| {
        write_wav(&wav, samples)?;
        let mut command = Command::new(&paths.executable);
        command.arg("-m").arg(&paths.model);
        if use_vad {
            command.arg("--vad").arg(
                paths
                    .vad
                    .as_ref()
                    .expect("SenseVoice always has a VAD model"),
            );
        }
        command
            .arg("-a")
            .arg(&wav)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(directory) = paths.executable.parent() {
            command.current_dir(directory);
        }
        configure_recognition_process(&mut command, background);
        let output = command.output().context("无法启动 SenseVoice 识别程序")?;
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr);
            bail!(
                "SenseVoice 识别失败（{}）：{}",
                output.status,
                detail.trim()
            );
        }
        let text = String::from_utf8(output.stdout).context("SenseVoice 输出不是 UTF-8")?;
        Ok(text.trim().to_owned())
    })();
    let _ = fs::remove_file(&wav);
    result
}

fn engine_files() -> Result<(PathBuf, PathBuf)> {
    let base = crate::project_directory()
        .join("assets")
        .join("voice")
        .join("whisper");
    let engine = base.join("whisper-cli.exe");
    if !engine.is_file() {
        bail!("缺少 {}", engine.display());
    }
    let model = [
        "ggml-medium.bin",
        "ggml-small.bin",
        "ggml-base.bin",
        "ggml-tiny.bin",
    ]
    .iter()
    .map(|name| base.join(name))
    .find(|path| path.is_file())
    .with_context(|| format!("缺少多语言模型，请将 ggml-base.bin 放在 {}", base.display()))?;
    Ok((engine, model))
}

fn whisper_language(language: &str) -> &str {
    match language {
        "cn" | "zh_tw" => "zh",
        "fil" => "tl",
        "auto" => "auto",
        other => other,
    }
}

fn transcribe(
    engine: &Path,
    model: &Path,
    samples: &[i16],
    language: &str,
    background: bool,
) -> Result<String> {
    let paths = voice_path_compat::prepare(engine, model, None)?;
    let temp_id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let prefix = paths
        .scratch
        .join(format!("to_words_voice_{}_{}", std::process::id(), temp_id));
    let wav = prefix.with_extension("wav");
    let output_text = prefix.with_extension("txt");
    let result = (|| {
        write_wav(&wav, samples)?;
        let mut command = Command::new(&paths.executable);
        command
            .arg("--model")
            .arg(&paths.model)
            .arg("--file")
            .arg(&wav)
            .arg("--language")
            .arg(language)
            .arg("--threads")
            .arg(whisper_thread_count(background).to_string())
            .arg("--output-txt")
            .arg("--output-file")
            .arg(&prefix)
            .arg("--no-timestamps")
            .arg("--no-prints")
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if let Some(directory) = paths.executable.parent() {
            command.current_dir(directory);
        }
        configure_recognition_process(&mut command, background);
        let output = command.output().context("无法启动本地语音识别引擎")?;
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr);
            bail!("语音识别引擎执行失败：{}", detail.trim());
        }
        let text = fs::read_to_string(&output_text).context("语音识别未生成文本结果")?;
        Ok(text.trim().to_string())
    })();
    let _ = fs::remove_file(&wav);
    let _ = fs::remove_file(&output_text);
    result
}

fn whisper_thread_count(background: bool) -> usize {
    let available = std::thread::available_parallelism()
        .map(|count| count.get().min(8))
        .unwrap_or(4);
    if background {
        available.div_ceil(2).min(4)
    } else {
        available
    }
}

fn configure_recognition_process(command: &mut Command, background: bool) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::{
            BELOW_NORMAL_PRIORITY_CLASS, CREATE_NO_WINDOW,
        };
        let priority = if background {
            BELOW_NORMAL_PRIORITY_CLASS
        } else {
            0
        };
        command.creation_flags(CREATE_NO_WINDOW | priority);
    }
    #[cfg(not(windows))]
    let _ = (command, background);
}

fn write_wav(path: &Path, samples: &[i16]) -> Result<()> {
    let data_bytes = u32::try_from(samples.len() * 2).context("录音过长")?;
    let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1_u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&(SAMPLE_RATE as u32).to_le_bytes());
    bytes.extend_from_slice(&((SAMPLE_RATE * 2) as u32).to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&16_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_bytes.to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    fs::write(path, bytes).with_context(|| format!("无法写入临时录音：{}", path.display()))
}

fn audio_duration(sample_count: usize) -> Duration {
    Duration::from_millis((sample_count as u64 * 1000) / SAMPLE_RATE as u64)
}

#[cfg(windows)]
pub(crate) fn record_microphone(
    stop: Arc<AtomicBool>,
    sender: mpsc::Sender<Vec<i16>>,
    keep_input: bool,
    trace: VoiceTrace,
) -> Result<()> {
    use windows_sys::Win32::Media::Audio::{
        CALLBACK_NULL, HWAVEIN, WAVE_FORMAT_PCM, WAVE_MAPPER, WAVEFORMATEX, WAVEHDR, WHDR_DONE,
        waveInAddBuffer, waveInClose, waveInOpen, waveInPrepareHeader, waveInReset, waveInStart,
        waveInUnprepareHeader,
    };

    const BUFFER_BYTES: usize = SAMPLE_RATE / 10 * 2;
    let format = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM as u16,
        nChannels: 1,
        nSamplesPerSec: SAMPLE_RATE as u32,
        nAvgBytesPerSec: (SAMPLE_RATE * 2) as u32,
        nBlockAlign: 2,
        wBitsPerSample: 16,
        cbSize: 0,
    };
    let open_started = Instant::now();
    let mut handle: HWAVEIN = std::ptr::null_mut();
    let open = unsafe { waveInOpen(&mut handle, WAVE_MAPPER, &format, 0, 0, CALLBACK_NULL) };
    if open != 0 {
        trace.record(0, "microphone_open", Some(open_started.elapsed()), "error");
        bail!("无法打开麦克风（Windows 错误 {open}），请检查麦克风权限与输入设备");
    }
    trace.record(0, "microphone_open", Some(open_started.elapsed()), "ok");

    // Keep enough buffers queued for brief CPU spikes (for example, local TTS
    // synthesis starting while continuous speech is being captured). Reusing
    // only three 100 ms buffers can make waveIn drop the middle of a sentence.
    let mut buffers = vec![vec![0_u8; BUFFER_BYTES]; 12];
    let mut headers: Vec<WAVEHDR> = buffers
        .iter_mut()
        .map(|buffer| WAVEHDR {
            lpData: buffer.as_mut_ptr(),
            dwBufferLength: BUFFER_BYTES as u32,
            dwBytesRecorded: 0,
            dwUser: 0,
            dwFlags: 0,
            dwLoops: 0,
            lpNext: std::ptr::null_mut(),
            reserved: 0,
        })
        .collect();
    let header_size = std::mem::size_of::<WAVEHDR>() as u32;
    let mut prepared = vec![false; buffers.len()];
    let mut total_samples = 0;
    let mut next_header = 0;
    let result = (|| {
        for (index, header) in headers.iter_mut().enumerate() {
            let code = unsafe { waveInPrepareHeader(handle, header, header_size) };
            if code != 0 {
                bail!("麦克风缓冲区初始化失败（Windows 错误 {code}）");
            }
            prepared[index] = true;
            let code = unsafe { waveInAddBuffer(handle, header, header_size) };
            if code != 0 {
                bail!("麦克风缓冲区提交失败（Windows 错误 {code}）");
            }
        }
        let code = unsafe { waveInStart(handle) };
        if code != 0 {
            bail!("无法开始录音（Windows 错误 {code}）");
        }
        trace.record(0, "microphone_ready", None, "ok");
        while !stop.load(Ordering::Acquire) {
            let index = next_header;
            if headers[index].dwFlags & WHDR_DONE == 0 {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            let recorded = (headers[index].dwBytesRecorded as usize).min(BUFFER_BYTES);
            if recorded >= 2 {
                let captured: Vec<i16> = buffers[index][..recorded]
                    .chunks_exact(2)
                    .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
                    .collect();
                total_samples += captured.len();
                let _ = sender.send(captured);
                if !keep_input && total_samples >= SAMPLE_RATE * MAX_SECONDS {
                    stop.store(true, Ordering::Release);
                }
            }
            let code = unsafe { waveInUnprepareHeader(handle, &mut headers[index], header_size) };
            if code != 0 {
                bail!("麦克风缓冲区释放失败（Windows 错误 {code}）");
            }
            prepared[index] = false;
            headers[index].dwFlags = 0;
            headers[index].dwBytesRecorded = 0;
            let code = unsafe { waveInPrepareHeader(handle, &mut headers[index], header_size) };
            if code != 0 {
                bail!("麦克风缓冲区重置失败（Windows 错误 {code}）");
            }
            prepared[index] = true;
            let code = unsafe { waveInAddBuffer(handle, &mut headers[index], header_size) };
            if code != 0 {
                bail!("麦克风缓冲区重提交失败（Windows 错误 {code}）");
            }
            next_header = (next_header + 1) % headers.len();
        }
        Ok(())
    })();
    unsafe {
        waveInReset(handle);
        for offset in 0..headers.len() {
            let index = (next_header + offset) % headers.len();
            let header = &mut headers[index];
            let was_prepared = prepared[index];
            if was_prepared {
                let recorded = (header.dwBytesRecorded as usize).min(BUFFER_BYTES);
                if recorded >= 2 {
                    let captured: Vec<i16> = buffers[index][..recorded]
                        .chunks_exact(2)
                        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
                        .collect();
                    let _ = sender.send(captured);
                }
                waveInUnprepareHeader(handle, header, header_size);
            }
        }
        waveInClose(handle);
    }
    result
}

#[cfg(not(windows))]
pub(crate) fn record_microphone(
    _stop: Arc<AtomicBool>,
    _sender: mpsc::Sender<Vec<i16>>,
    _keep_input: bool,
    _trace: VoiceTrace,
) -> Result<()> {
    bail!("语音识别目前仅支持 Windows")
}

#[cfg(test)]
mod tests {
    use super::{
        SAMPLE_RATE, SpeechGate, effective_voice_backend, effective_voice_language, engine_files,
        sensevoice_files_in, transcribe, whisper_language, whisper_thread_count, write_wav,
    };

    #[test]
    fn language_selection_preserves_the_selected_engine() {
        assert_eq!(effective_voice_language("auto", "ko", "ko"), "ko");
        assert_eq!(effective_voice_language("auto", "en", "en"), "en");
        assert_eq!(effective_voice_language("auto", "ja", "ja"), "ja");
        assert_eq!(effective_voice_language("auto", "cn", "ko"), "auto");
        assert_eq!(effective_voice_language("ja", "ko", "ko"), "ko");
        assert_eq!(
            effective_voice_backend("sensevoice", "ko", "ko"),
            "sensevoice"
        );
        assert_eq!(
            effective_voice_backend("sensevoice", "en", "en"),
            "sensevoice"
        );
        assert_eq!(
            effective_voice_backend("sensevoice", "ja", "ja"),
            "sensevoice"
        );
        assert_eq!(
            effective_voice_backend("sensevoice", "cn", "cn"),
            "sensevoice"
        );
        assert_eq!(effective_voice_backend("whisper", "cn", "cn"), "whisper");
        assert_eq!(
            effective_voice_backend("sensevoice", "cn", "ko"),
            "sensevoice"
        );
    }

    #[test]
    fn voice_language_uses_whisper_codes() {
        assert_eq!(whisper_language("cn"), "zh");
        assert_eq!(whisper_language("zh_tw"), "zh");
        assert_eq!(whisper_language("fil"), "tl");
        assert_eq!(whisper_language("ko"), "ko");
        assert_eq!(whisper_language("auto"), "auto");
    }

    #[test]
    fn whisper_threads_respect_cpu_limit() {
        let one_shot = whisper_thread_count(false);
        let continuous = whisper_thread_count(true);
        assert!((1..=8).contains(&one_shot));
        assert!((1..=4).contains(&continuous));
        assert!(continuous <= one_shot);
    }

    #[test]
    fn sensevoice_reports_missing_model_file() {
        let missing = std::env::temp_dir().join(format!(
            "to_words_missing_sensevoice_{}",
            std::process::id()
        ));
        let error = sensevoice_files_in(&missing).unwrap_err().to_string();
        assert!(error.contains("llama-funasr-sensevoice.exe"));
    }

    #[test]
    fn bundled_sensevoice_supports_language_query_override() {
        let Ok((_, model, _)) = super::sensevoice_files() else {
            return; // Optional model files are not checked into source control.
        };
        let (_, token) =
            super::query_language_token(&mut std::fs::File::open(model).unwrap()).unwrap();
        assert_eq!(token, 0);
        assert_eq!(super::sensevoice_language_token("ko"), Some(12));
        assert_eq!(super::sensevoice_language_token("ja"), Some(11));
        assert_eq!(super::sensevoice_language_token("en"), Some(4));
    }

    #[test]
    #[ignore = "copies the optional 254 MB model and starts the SenseVoice executable"]
    fn forced_subtitle_model_loads_without_changing_the_original() {
        let (executable, model, vad) = super::sensevoice_files().unwrap();
        let korean = super::prepare_subtitle_language_model(&model, "ko").unwrap();
        assert_eq!(
            super::query_language_token(&mut std::fs::File::open(&korean).unwrap())
                .unwrap()
                .1,
            12
        );
        assert_eq!(
            super::query_language_token(&mut std::fs::File::open(&model).unwrap())
                .unwrap()
                .1,
            0
        );
        super::transcribe_sensevoice(
            &executable,
            &korean,
            &vad,
            &vec![0; SAMPLE_RATE],
            true,
            false,
        )
        .unwrap();
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires a Korean speech source currently playing on the default Windows output"]
    fn live_korean_playback_uses_hangul() {
        let mut session = super::start_playback("sensevoice", 0.6, "", "ko").unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let mut recognized = 0;
        let mut korean = 0;
        while std::time::Instant::now() < deadline {
            if let Ok(super::VoiceEvent::Utterance(_, Ok(text))) = session
                .receiver
                .recv_timeout(std::time::Duration::from_millis(500))
            {
                recognized += 1;
                korean += usize::from(text.chars().any(|ch| matches!(ch, '\u{AC00}'..='\u{D7A3}')));
            }
        }
        session.stop();
        assert!(recognized > 0, "没有从当前播放设备获得可识别语音");
        assert!(korean > 0, "当前播放内容没有识别出韩文");
    }

    #[test]
    #[ignore = "requires the local whisper.cpp binary and model"]
    fn benchmark_local_engine_startup() {
        let (engine, model) = engine_files().unwrap();
        let silent_audio = vec![0_i16; SAMPLE_RATE * 2];
        let started = std::time::Instant::now();
        let _ = transcribe(&engine, &model, &silent_audio, "auto", false).unwrap();
        eprintln!("本地两秒音频识别耗时：{:?}", started.elapsed());
    }

    #[test]
    fn voice_gate_ends_after_six_tenths_of_a_second_of_silence() {
        let mut gate = SpeechGate::new(0.6, true);
        let silence = vec![0_i16; SAMPLE_RATE / 10];
        let speech = vec![2_000_i16; SAMPLE_RATE / 10];
        for _ in 0..4 {
            assert!(!gate.push(&silence).started);
        }
        assert!(!gate.push(&speech).started);
        assert!(gate.push(&speech).started);
        for _ in 0..5 {
            assert!(gate.push(&speech).completed.is_none());
        }
        for _ in 0..5 {
            assert!(gate.push(&silence).completed.is_none());
        }
        let completed = gate.push(&silence).completed.unwrap();
        assert!(completed.samples.len() >= SAMPLE_RATE / 2);
        assert_eq!(completed.trailing_silence.as_millis(), 600);
        assert_eq!(completed.reason, "silence");
        assert!(!gate.push(&speech).started);
        assert!(gate.push(&speech).started);
    }

    #[test]
    fn playback_gate_starts_during_existing_audio_and_submits_without_a_pause() {
        let mut gate = SpeechGate::new_playback(0.6);
        let playback = vec![1_000_i16; SAMPLE_RATE / 10];
        assert!(!gate.push(&playback).started);
        assert!(gate.push(&playback).started);
        let mut completed = None;
        for _ in 0..50 {
            if let Some(segment) = gate.push(&playback).completed {
                completed = Some(segment);
                break;
            }
        }
        let completed = completed.expect("continuous playback should produce a subtitle segment");
        assert_eq!(completed.reason, "max_duration");
        assert!(completed.samples.len() <= SAMPLE_RATE * 5 + playback.len());
        assert!(!gate.push(&playback).started);
        assert!(gate.push(&playback).started);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "plays the checked-in voice sample and runs the installed SenseVoice model"]
    fn playback_sample_reaches_sensevoice() {
        let mut session = super::start_playback("sensevoice", 0.6, "", "auto").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let sample = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/voice/test_samples/test-voice2.wav");
        let playable =
            std::env::temp_dir().join(format!("to_words_playback_{}.wav", std::process::id()));
        let converted = std::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(&sample)
            .args(["-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le"])
            .arg(&playable)
            .status()
            .expect("ffmpeg installed for playback test");
        assert!(converted.success());
        let output = std::process::Command::new("powershell")
            .arg("-NoProfile")
            .arg("-Command")
            .arg("(New-Object System.Media.SoundPlayer $env:TO_WORDS_TEST_WAV).PlaySync()")
            .env("TO_WORDS_TEST_WAV", &playable)
            .output()
            .expect("sample playback should start");
        assert!(output.status.success(), "sample playback should succeed");
        let _ = std::fs::remove_file(playable);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut recognized = None;
        let mut partial = None;
        while std::time::Instant::now() < deadline {
            match session
                .receiver
                .recv_timeout(std::time::Duration::from_millis(250))
            {
                Ok(super::VoiceEvent::Partial(_, text)) if !text.trim().is_empty() => {
                    partial = Some(text);
                }
                Ok(super::VoiceEvent::Utterance(_, result)) => {
                    recognized = Some(result);
                    break;
                }
                Ok(super::VoiceEvent::Stopped(reason)) => {
                    panic!("playback recognition stopped: {reason:?}")
                }
                _ => {}
            }
        }
        session.stop();
        let text = recognized
            .expect("playback recognition produced no utterance")
            .unwrap();
        assert!(
            partial.is_some(),
            "streaming recognizer produced no early subtitle"
        );
        assert!(
            !text.trim().is_empty(),
            "SenseVoice returned no subtitle text"
        );
        assert!(
            !crate::domain::voice_word_cache::is_obviously_bad(&text),
            "valid playback speech should not be discarded by subtitle filtering"
        );
    }

    #[test]
    fn configured_voice_silence_threshold_is_used() {
        assert_eq!(
            SpeechGate::new(1.2, true).silence_threshold_samples,
            SAMPLE_RATE * 12 / 10
        );
        assert_eq!(
            SpeechGate::new(0.1, true).silence_threshold_samples,
            SAMPLE_RATE / 5
        );
        assert_eq!(
            SpeechGate::new(f32::NAN, true).silence_threshold_samples,
            SAMPLE_RATE * 6 / 10
        );
    }

    #[test]
    fn push_to_talk_keeps_recording_through_a_pause_until_release() {
        let mut gate = SpeechGate::new(0.6, false);
        let silence = vec![0_i16; SAMPLE_RATE / 10];
        let speech = vec![2_000_i16; SAMPLE_RATE / 10];
        for _ in 0..4 {
            gate.push(&silence);
        }
        gate.push(&speech);
        assert!(gate.push(&speech).started);
        for _ in 0..5 {
            gate.push(&speech);
        }
        for _ in 0..10 {
            assert!(gate.push(&silence).completed.is_none());
        }
        assert!(gate.take_active(SAMPLE_RATE / 5).is_some());
    }

    #[test]
    fn steady_background_noise_does_not_prevent_auto_submit() {
        let mut gate = SpeechGate::new(0.6, true);
        let noise = vec![700_i16; SAMPLE_RATE / 10];
        let speech = vec![2_000_i16; SAMPLE_RATE / 10];
        for _ in 0..10 {
            assert!(!gate.push(&noise).started);
        }
        assert!(!gate.push(&speech).started);
        assert!(gate.push(&speech).started);
        for _ in 0..5 {
            assert!(gate.push(&speech).completed.is_none());
        }
        for _ in 0..5 {
            assert!(gate.push(&noise).completed.is_none());
        }
        assert!(gate.push(&noise).completed.is_some());
    }

    #[test]
    fn noise_after_quiet_start_counts_as_silence_after_speech() {
        let mut gate = SpeechGate::new(0.6, true);
        let quiet = vec![0_i16; SAMPLE_RATE / 10];
        let noise = vec![700_i16; SAMPLE_RATE / 10];
        let speech = vec![2_000_i16; SAMPLE_RATE / 10];
        for _ in 0..4 {
            gate.push(&quiet);
        }
        gate.push(&speech);
        assert!(gate.push(&speech).started);
        for _ in 0..5 {
            gate.push(&speech);
        }
        for _ in 0..5 {
            assert!(gate.push(&noise).completed.is_none());
        }
        assert!(gate.push(&noise).completed.is_some());
    }

    #[test]
    fn quieter_words_after_a_loud_syllable_remain_in_the_same_utterance() {
        let mut gate = SpeechGate::new(0.6, true);
        let ambient = vec![700_i16; SAMPLE_RATE / 10];
        let loud = vec![4_000_i16; SAMPLE_RATE / 10];
        let quiet_speech = vec![1_000_i16; SAMPLE_RATE / 10];
        for _ in 0..4 {
            gate.push(&ambient);
        }
        gate.push(&loud);
        assert!(gate.push(&loud).started);
        for _ in 0..10 {
            assert!(gate.push(&quiet_speech).completed.is_none());
        }
        for _ in 0..5 {
            assert!(gate.push(&ambient).completed.is_none());
        }
        assert!(gate.push(&ambient).completed.is_some());
    }

    #[test]
    fn short_pause_resume_keeps_the_first_words_for_recognition() {
        let mut gate = SpeechGate::new(0.6, true);
        let ambient = vec![0_i16; SAMPLE_RATE / 10];
        let first_word = vec![2_000_i16; SAMPLE_RATE / 10];
        let next_words = vec![1_000_i16; SAMPLE_RATE / 10];
        for _ in 0..4 {
            gate.push(&ambient);
        }
        gate.push(&first_word);
        assert!(gate.push(&first_word).started);
        let first = (0..6).find_map(|_| gate.push(&ambient).completed).unwrap();
        assert!(first.samples.contains(&2_000));
        gate.push(&next_words);
        assert!(gate.push(&next_words).started);
        gate.resume_short_segment(first);
        let combined = gate.take_active(SAMPLE_RATE / 5).unwrap();
        assert!(combined.contains(&2_000));
        assert!(combined.contains(&1_000));
        assert!(
            combined.iter().position(|sample| *sample == 2_000)
                < combined.iter().position(|sample| *sample == 1_000)
        );
    }

    #[test]
    fn fluctuating_background_noise_still_ends_utterance() {
        let mut gate = SpeechGate::new(0.6, true);
        let low_noise = vec![650_i16; SAMPLE_RATE / 10];
        let high_noise = vec![750_i16; SAMPLE_RATE / 10];
        let speech = vec![2_000_i16; SAMPLE_RATE / 10];
        for index in 0..8 {
            assert!(
                !gate
                    .push(if index % 2 == 0 {
                        &low_noise
                    } else {
                        &high_noise
                    })
                    .started
            );
        }
        gate.push(&speech);
        assert!(gate.push(&speech).started);
        for _ in 0..5 {
            gate.push(&speech);
        }
        for index in 0..5 {
            assert!(
                gate.push(if index % 2 == 0 {
                    &low_noise
                } else {
                    &high_noise
                })
                .completed
                .is_none()
            );
        }
        assert!(gate.push(&low_noise).completed.is_some());
    }

    #[test]
    fn long_utterance_reports_timeout_instead_of_silence() {
        let mut gate = SpeechGate::new(0.6, true);
        let quiet = vec![0_i16; SAMPLE_RATE / 10];
        let speech = vec![2_000_i16; SAMPLE_RATE / 10];
        for _ in 0..4 {
            gate.push(&quiet);
        }
        gate.push(&speech);
        assert!(gate.push(&speech).started);
        let long_speech = vec![2_000_i16; SAMPLE_RATE * super::MAX_UTTERANCE_SECONDS];
        let completed = gate.push(&long_speech).completed.unwrap();
        assert_eq!(completed.reason, "max_duration");
    }

    #[test]
    fn wav_header_is_pcm_16khz_mono() {
        let path = std::env::temp_dir().join(format!("to_words_test_{}.wav", std::process::id()));
        write_wav(&path, &[1, -1]).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(path);
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(
            u32::from_le_bytes(bytes[24..28].try_into().unwrap()),
            16_000
        );
        assert_eq!(&bytes[44..48], &[1, 0, 255, 255]);
    }
}
