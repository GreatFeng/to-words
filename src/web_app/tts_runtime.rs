//! 查询和语音结果共用的自动发音队列。

use super::*;
use crate::platform::{tts_output, windows_tts};

pub(super) struct TtsRequest {
    text: String,
    config: UiConfig,
    generation: u64,
}

struct AudioJob {
    text: String,
    device_id: String,
    samples: Vec<f32>,
    sample_rate: u32,
    generation: u64,
}

pub(super) struct RecentTts {
    text: String,
    started: Instant,
    finished: Option<Instant>,
    generation: u64,
}

fn normalized_speech(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .filter(|ch| ch.is_alphanumeric())
        .collect()
}

fn matches_self_tts_echo(playback: &RecentTts, recognized: &str, speech_started: Instant) -> bool {
    let played = normalized_speech(&playback.text);
    if played.chars().count() < 8 {
        return false;
    }
    let recognized = normalized_speech(recognized);
    let extra_chars = recognized
        .chars()
        .count()
        .saturating_sub(played.chars().count());
    if !recognized.starts_with(&played) || extra_chars > played.chars().count().max(24) / 3 {
        return false;
    }
    let playback_end = playback.finished.unwrap_or_else(Instant::now);
    speech_started + Duration::from_millis(200) >= playback.started
        && speech_started <= playback_end + Duration::from_millis(600)
        && playback
            .finished
            .is_none_or(|finished| finished.elapsed() <= Duration::from_secs(4))
}

pub(super) fn is_recent_tts_echo(state: &WebState, recognized: &str, started: Instant) -> bool {
    state.recent_tts.lock().is_ok_and(|recent| {
        recent
            .iter()
            .any(|playback| matches_self_tts_echo(playback, recognized, started))
    })
}

pub(super) enum TtsCommand {
    Speak(TtsRequest),
    ListVoices(Sender<Result<Vec<windows_tts::WindowsTtsVoice>, String>>),
    Synthesize {
        voice_id: String,
        text: String,
        reply: Sender<Result<(Vec<f32>, u32), String>>,
    },
}

fn speech_language(config: &UiConfig, text: &str) -> String {
    if text.chars().any(|c| ('가'..='힣').contains(&c)) {
        return "ko-kr".into();
    }
    if text.chars().any(|c| ('ぁ'..='ヿ').contains(&c)) {
        return "ja-jp".into();
    }
    if text.chars().any(|c| ('一'..='鿿').contains(&c))
        && !matches!(config.translation_language.as_str(), "cn" | "zh_tw" | "ja")
    {
        return "zh-cn".into();
    }
    if text.chars().any(|c| c.is_ascii_alphabetic())
        && !text.chars().any(|c| ('一'..='鿿').contains(&c))
        && matches!(
            config.translation_language.as_str(),
            "cn" | "zh_tw" | "ko" | "ja"
        )
    {
        return "en-us".into();
    }
    let code = config.translation_language.as_str();
    match code {
        "cn" => "zh-cn".into(),
        "zh_tw" => "zh-tw".into(),
        "ko" => "ko-kr".into(),
        "ja" => "ja-jp".into(),
        "en" => "en-us".into(),
        _ if !code.is_empty() => code.to_ascii_lowercase(),
        _ if text.chars().any(|c| ('一'..='鿿').contains(&c)) => "zh-cn".into(),
        _ => "en-us".into(),
    }
}

fn selected_voice(config: &UiConfig, text: &str) -> Result<String, String> {
    let language = speech_language(config, text);
    let primary = language.split('-').next().unwrap_or("");
    let selected = config
        .speech_voices
        .get(&language)
        .or_else(|| config.speech_voices.get(primary))
        .cloned()
        .unwrap_or_default();
    if selected.starts_with("piper:")
        || selected.starts_with("supertonic:")
        || selected.starts_with("winrt:")
    {
        return Ok(selected);
    }
    // WebView 的 voiceURI 只用于本机试听。旧配置可能保存了这个 ID；
    // 自动麦克风发音优先查找同一个 Windows 声音，再按语言选择本地声音。
    if !selected.is_empty() {
        if let Ok(windows) = windows_tts::list_voices() {
            let selected_lower = selected.to_ascii_lowercase();
            if let Some(voice) = windows.iter().find(|voice| {
                let id = voice.id.to_ascii_lowercase();
                let name = voice.name.to_ascii_lowercase().replace(" · windows", "");
                (id.contains(&selected_lower) || selected_lower.contains(&name))
                    && voice.lang.to_ascii_lowercase().starts_with(primary)
            }) {
                return Ok(voice.id.clone());
            }
        }
    }
    let local = piper_tts::list_voices();
    if let Some(voice) = local
        .iter()
        .find(|voice| voice.lang.to_ascii_lowercase() == language)
        .or_else(|| {
            local
                .iter()
                .find(|voice| voice.lang.to_ascii_lowercase().starts_with(primary))
        })
    {
        return Ok(voice.id.clone());
    }
    let windows = windows_tts::list_voices().map_err(error)?;
    windows
        .iter()
        .find(|voice| voice.lang.to_ascii_lowercase() == language)
        .or_else(|| {
            windows
                .iter()
                .find(|voice| voice.lang.to_ascii_lowercase().starts_with(primary))
        })
        .map(|voice| voice.id.clone())
        .ok_or_else(|| format!("未找到 {language} 的本地发音语音，请在设置中选择模型"))
}

pub(super) fn queue_tts(state: &WebState, config: &UiConfig, text: &str) -> Result<(), String> {
    if !config.mic_tts_enabled || text.trim().is_empty() {
        return Ok(());
    }
    if config.mic_output_device.is_empty() {
        return Err("请先在设置中选择虚拟音频播放设备".into());
    }
    let request = TtsRequest {
        text: text.trim().to_owned(),
        config: config.clone(),
        generation: state.tts_generation.fetch_add(1, Ordering::Relaxed) + 1,
    };
    state
        .tts_control
        .lock()
        .map_err(error)?
        .as_ref()
        .ok_or("发音服务未启动")?
        .send(TtsCommand::Speak(request))
        .map_err(error)
}

pub(super) fn start_tts_worker(
    app: AppHandle,
    receiver: Receiver<TtsCommand>,
    generation: Arc<AtomicU64>,
) {
    std::thread::spawn(move || {
        let (audio_tx, audio_rx) = mpsc::channel::<AudioJob>();
        let audio_app = app.clone();
        let audio_generation = generation.clone();
        std::thread::spawn(move || {
            for mut job in &audio_rx {
                // 持续语音时，跳过播放期间已经过时的译文。
                for newer in audio_rx.try_iter() {
                    job = newer;
                }
                if job.generation != audio_generation.load(Ordering::Relaxed) {
                    continue;
                }
                if let Ok(mut recent) = audio_app.state::<WebState>().recent_tts.lock() {
                    recent.retain(|playback| {
                        playback
                            .finished
                            .is_none_or(|at| at.elapsed() < Duration::from_secs(4))
                    });
                    recent.push_back(RecentTts {
                        text: job.text.clone(),
                        started: Instant::now(),
                        finished: None,
                        generation: job.generation,
                    });
                    while recent.len() > 4 {
                        recent.pop_front();
                    }
                }
                let played =
                    tts_output::play_samples(&job.device_id, &job.samples, job.sample_rate, || {
                        job.generation != audio_generation.load(Ordering::Relaxed)
                    });
                if let Ok(mut recent) = audio_app.state::<WebState>().recent_tts.lock()
                    && let Some(playback) = recent
                        .iter_mut()
                        .find(|playback| playback.generation == job.generation)
                {
                    playback.finished = Some(Instant::now());
                }
                if let Err(reason) = played {
                    report_error(&audio_app, reason);
                }
            }
        });
        for command in receiver {
            match command {
                TtsCommand::ListVoices(reply) => {
                    let _ = reply.send(windows_tts::list_voices().map_err(error));
                }
                TtsCommand::Synthesize {
                    voice_id,
                    text,
                    reply,
                } => {
                    let _ = reply.send(windows_tts::synthesize(&voice_id, &text).map_err(error));
                }
                TtsCommand::Speak(request)
                    if request.generation != generation.load(Ordering::Relaxed) => {}
                TtsCommand::Speak(request) => match synthesize_request(request) {
                    Ok(job) if job.generation == generation.load(Ordering::Relaxed) => {
                        let _ = audio_tx.send(job);
                    }
                    Ok(_) => {}
                    Err(reason) => report_error(&app, reason),
                },
            }
        }
    });
}

fn synthesize_request(request: TtsRequest) -> anyhow::Result<AudioJob> {
    let voice_id = selected_voice(&request.config, &request.text).map_err(anyhow::Error::msg)?;
    let (samples, sample_rate) =
        if voice_id.starts_with("piper:") || voice_id.starts_with("supertonic:") {
            piper_tts::synthesize_samples(&voice_id, &request.text)?
        } else {
            windows_tts::synthesize(&voice_id, &request.text)?
        };
    Ok(AudioJob {
        text: request.text,
        device_id: request.config.mic_output_device,
        samples,
        sample_rate,
        generation: request.generation,
    })
}

fn report_error(app: &AppHandle, reason: anyhow::Error) {
    eprintln!("自动麦克风发音失败：{reason:#}");
    use tauri::Emitter;
    let message = format!("自动麦克风发音失败：{reason:#}");
    let _ = app.emit_to("query", "mic-tts-status", &message);
    let _ = app.emit_to("settings", "mic-tts-status", &message);
}

pub(super) fn change_mic_tts(
    app: &AppHandle,
    enabled: Option<bool>,
    output_device: Option<String>,
) -> Result<bool, String> {
    use tauri::Emitter;

    let state = app.state::<WebState>();
    let mut config = state.config.lock().map_err(error)?;
    let next_enabled = enabled.unwrap_or(!config.mic_tts_enabled);
    let mut next = config.clone();
    if let Some(device) = output_device {
        next.mic_output_device = device;
    }
    if next_enabled && next.mic_output_device.is_empty() {
        return Err("请先在设置中选择虚拟音频播放设备".into());
    }
    next.mic_tts_enabled = next_enabled;
    save_ui_config(&next).map_err(error)?;
    *config = next;
    drop(config);
    if !next_enabled {
        state.tts_generation.fetch_add(1, Ordering::Relaxed);
    }
    let status = if next_enabled {
        "自动麦克风发音已开启"
    } else {
        "自动麦克风发音已关闭"
    };
    if let Some(tray) = app.tray_by_id("to_words_tray") {
        let _ = tray.set_tooltip(Some(format!("to_words · {status}")));
    }
    let _ = app.emit_to("settings", "mic-tts-toggle", next_enabled);
    let _ = app.emit_to("query", "mic-tts-toggle", next_enabled);
    let _ = app.emit_to("settings", "mic-tts-status", status);
    let _ = app.emit_to("query", "mic-tts-status", status);
    Ok(next_enabled)
}

#[tauri::command]
pub(super) fn set_mic_tts_enabled(
    app: AppHandle,
    enabled: bool,
    output_device: Option<String>,
) -> Result<bool, String> {
    change_mic_tts(&app, Some(enabled), output_device)
}

pub(super) fn toggle_mic_tts(app: &AppHandle) {
    if let Err(reason) = change_mic_tts(app, None, None) {
        use tauri::Emitter;
        eprintln!("切换自动麦克风发音失败：{reason}");
        let _ = app.emit_to("settings", "mic-tts-status", &reason);
        let _ = app.emit_to("query", "mic-tts-status", &reason);
    }
}

#[cfg(test)]
mod tests {
    use super::{RecentTts, matches_self_tts_echo, selected_voice, speech_language};
    use crate::web_config::UiConfig;
    use std::time::{Duration, Instant};

    #[test]
    fn speaker_translation_with_hallucinated_suffix_is_filtered() {
        let started = Instant::now();
        let playback = RecentTts {
            text: "Please briefly introduce yourself.".into(),
            started,
            finished: Some(started + Duration::from_secs(2)),
            generation: 1,
        };
        assert!(matches_self_tts_echo(
            &playback,
            "Please briefly introduce yourself.请呼吸。",
            started + Duration::from_millis(300),
        ));
        assert!(!matches_self_tts_echo(
            &playback,
            "Could you introduce yourself?",
            started + Duration::from_millis(300),
        ));
        assert!(!matches_self_tts_echo(
            &playback,
            "Please briefly introduce yourself. Now I want to start a new game.",
            started + Duration::from_millis(300),
        ));
        assert!(!matches_self_tts_echo(
            &playback,
            "Please briefly introduce yourself.",
            started - Duration::from_secs(1),
        ));
    }

    #[test]
    fn microphone_speech_uses_the_configured_voice_for_the_result_language() {
        let mut config = UiConfig::default();
        config.translation_language = "ko".into();
        config
            .speech_voices
            .insert("ko-kr".into(), "piper:korean/model.onnx".into());
        assert_eq!(speech_language(&config, "안녕하세요"), "ko-kr");
        assert_eq!(
            selected_voice(&config, "안녕하세요").unwrap(),
            "piper:korean/model.onnx"
        );
    }
}
