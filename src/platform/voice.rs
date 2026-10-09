//! 本地语音识别：Windows 麦克风录音，默认使用 SenseVoice，也支持 Whisper 和实验版 Windows AI。
//!
//! 录音始终在后台线程进行。连续两帧检测到语音后开始收集句子，
//! 约 0.6 秒静音后自动提交；保持输入时继续收集下一句。

use crate::i18n::{self, tr};
use crate::platform::voice_metrics::VoiceTrace;
use anyhow::{Context, Result, bail};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

const SAMPLE_RATE: usize = 16_000;
const MAX_SECONDS: usize = 90;
const MAX_UTTERANCE_SECONDS: usize = 30;
const CALIBRATION_FRAMES: usize = 4;
static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) enum VoiceEvent {
    SpeechStarted(u64),
    Recognizing(u64),
    Utterance(u64, Result<String, String>),
    Stopped(Option<String>),
}

pub(crate) struct VoiceSession {
    stop: Arc<AtomicBool>,
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
}

pub(crate) fn engine_status(backend: &str) -> String {
    match selected_engine(backend) {
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
    }
}

pub(crate) fn start(
    keep_input: bool,
    push_to_talk: bool,
    language: &str,
    backend: &str,
    silence_seconds: f32,
) -> Result<VoiceSession> {
    let engine = selected_engine(backend)?;
    let language = whisper_language(language).to_string();
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
        trace.record(0, "asr_threads", None, &whisper_thread_count().to_string());
    }
    #[cfg(not(windows))]
    bail!("语音识别目前仅支持 Windows");

    let stop = Arc::new(AtomicBool::new(false));
    let (sender, receiver) = mpsc::channel();
    let worker_stop = Arc::clone(&stop);
    let worker_trace = trace.clone();
    std::thread::spawn(move || {
        let (audio_sender, audio_receiver) = mpsc::channel();
        let (segment_sender, segment_receiver) = mpsc::channel::<(u64, Vec<i16>)>();
        let asr_sender = sender.clone();
        let asr_trace = worker_trace.clone();
        let asr_stop = Arc::clone(&worker_stop);
        let transcriber = std::thread::spawn(move || {
            for (utterance_id, samples) in segment_receiver {
                let asr_started = Instant::now();
                asr_trace.record(utterance_id, "asr_started", None, "ok");
                let result = transcribe_selected(&engine, &samples, &language)
                    .map_err(|error| format!("{error:#}"));
                asr_trace.record(
                    utterance_id,
                    "asr_completed",
                    Some(asr_started.elapsed()),
                    if result.is_ok() { "ok" } else { "error" },
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
            record_microphone(recorder_stop, audio_sender, keep_input, recorder_trace)
        });
        let mut gate = SpeechGate::new(silence_seconds, !push_to_talk);
        let mut auto_stopped = false;
        let mut heard_speech = false;
        let mut utterance_id = 0;
        let started_at = Instant::now();
        loop {
            if !keep_input && !heard_speech && started_at.elapsed() >= Duration::from_secs(15) {
                worker_stop.store(true, Ordering::Release);
                worker_trace.record(0, "no_speech_timeout", None, "timeout");
                break;
            }
            match audio_receiver.recv_timeout(Duration::from_millis(50)) {
                Ok(chunk) => {
                    let update = gate.push(&chunk);
                    if update.started {
                        heard_speech = true;
                        utterance_id += 1;
                        worker_trace.record(utterance_id, "speech_started", None, "ok");
                        let _ = sender.send(VoiceEvent::SpeechStarted(utterance_id));
                    }
                    if let Some(segment) = update.completed {
                        if segment.reason == "silence" {
                            worker_trace.speech_end(utterance_id, segment.trailing_silence);
                        }
                        worker_trace.record(
                            utterance_id,
                            "vad_complete",
                            Some(segment.trailing_silence),
                            segment.reason,
                        );
                        worker_trace.record(
                            utterance_id,
                            "audio_segment",
                            Some(audio_duration(segment.samples.len())),
                            "ok",
                        );
                        let _ = sender.send(VoiceEvent::Recognizing(utterance_id));
                        if !keep_input {
                            worker_stop.store(true, Ordering::Release);
                            auto_stopped = true;
                        }
                        if segment_sender
                            .send((utterance_id, segment.samples))
                            .is_err()
                        {
                            let _ = sender.send(VoiceEvent::Utterance(
                                utterance_id,
                                Err("语音识别线程已停止".to_string()),
                            ));
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
        }
        worker_stop.store(true, Ordering::Release);
        let recorded = recorder
            .join()
            .map_err(|_| "麦克风录音线程意外终止".to_string())
            .and_then(|result| result.map_err(|error| format!("{error:#}")));
        // 手动停止时还没等到静音，仍提交最后一句；自动停止已经提交过，不再重复识别。
        let min_final_samples = if push_to_talk {
            SAMPLE_RATE / 5
        } else {
            SAMPLE_RATE / 2
        };
        if !auto_stopped && let Some(segment) = gate.take_active(min_final_samples) {
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
        trace,
        receiver,
        stopping: false,
    })
}

struct SpeechGate {
    noise_floor: f32,
    calibration: [f32; CALIBRATION_FRAMES],
    calibrated_frames: usize,
    speech_peak: f32,
    candidate_frames: u8,
    active: bool,
    silent_samples: usize,
    silence_threshold_samples: usize,
    submit_on_silence: bool,
    preroll: Vec<i16>,
    segment: Vec<i16>,
}

struct GateUpdate {
    started: bool,
    completed: Option<CompletedSegment>,
}

struct CompletedSegment {
    samples: Vec<i16>,
    trailing_silence: Duration,
    reason: &'static str,
}

impl SpeechGate {
    fn new(silence_seconds: f32, submit_on_silence: bool) -> Self {
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
            preroll: Vec::new(),
            segment: Vec::new(),
        }
    }

    fn push(&mut self, chunk: &[i16]) -> GateUpdate {
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
            let threshold = (self.noise_floor * 1.35)
                .max(self.noise_floor + 120.0)
                .max(240.0);
            if rms >= threshold {
                self.candidate_frames = self.candidate_frames.saturating_add(1);
            } else {
                self.candidate_frames = 0;
                self.noise_floor = self.noise_floor * 0.95 + rms * 0.05;
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
        let silence_threshold = (self.noise_floor * 1.2)
            .max(self.speech_peak * 0.62)
            .max(240.0);
        if rms < silence_threshold {
            self.silent_samples += chunk.len();
        } else {
            self.silent_samples = 0;
        }
        if (self.submit_on_silence && self.silent_samples >= self.silence_threshold_samples)
            || self.segment.len() >= SAMPLE_RATE * MAX_UTTERANCE_SECONDS
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
) -> Result<String> {
    match engine {
        RecognitionEngine::SenseVoice(executable, model, vad) => {
            transcribe_sensevoice(executable, model, vad, samples)
        }
        RecognitionEngine::Whisper(executable, model) => {
            transcribe(executable, model, samples, language)
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

fn transcribe_sensevoice(
    executable: &Path,
    model: &Path,
    vad: &Path,
    samples: &[i16],
) -> Result<String> {
    let temp_id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let wav = std::env::temp_dir().join(format!(
        "to_words_sensevoice_{}_{}.wav",
        std::process::id(),
        temp_id
    ));
    let result = (|| {
        write_wav(&wav, samples)?;
        let mut command = Command::new(executable);
        command
            .arg("-m")
            .arg(model)
            .arg("--vad")
            .arg(vad)
            .arg("-a")
            .arg(&wav)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(directory) = executable.parent() {
            command.current_dir(directory);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
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

fn transcribe(engine: &Path, model: &Path, samples: &[i16], language: &str) -> Result<String> {
    let temp_id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
    let prefix =
        std::env::temp_dir().join(format!("to_words_voice_{}_{}", std::process::id(), temp_id));
    let wav = prefix.with_extension("wav");
    let output_text = prefix.with_extension("txt");
    let result = (|| {
        write_wav(&wav, samples)?;
        let mut command = Command::new(engine);
        command
            .arg("--model")
            .arg(model)
            .arg("--file")
            .arg(&wav)
            .arg("--language")
            .arg(language)
            .arg("--threads")
            .arg(whisper_thread_count().to_string())
            .arg("--output-txt")
            .arg("--output-file")
            .arg(&prefix)
            .arg("--no-timestamps")
            .arg("--no-prints")
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if let Some(directory) = engine.parent() {
            command.current_dir(directory);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
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

fn whisper_thread_count() -> usize {
    std::thread::available_parallelism()
        .map(|count| count.get().min(8))
        .unwrap_or(4)
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
fn record_microphone(
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

    let mut buffers = vec![vec![0_u8; BUFFER_BYTES]; 3];
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
    let mut prepared = [false; 3];
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
fn record_microphone(
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
        SAMPLE_RATE, SpeechGate, engine_files, sensevoice_files_in, transcribe, whisper_language,
        whisper_thread_count, write_wav,
    };

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
        assert!((1..=8).contains(&whisper_thread_count()));
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
    #[ignore = "requires the local whisper.cpp binary and model"]
    fn benchmark_local_engine_startup() {
        let (engine, model) = engine_files().unwrap();
        let silent_audio = vec![0_i16; SAMPLE_RATE * 2];
        let started = std::time::Instant::now();
        let _ = transcribe(&engine, &model, &silent_audio, "auto").unwrap();
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
