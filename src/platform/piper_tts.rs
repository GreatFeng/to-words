//! 发现并播放放在 assets/tts 下的 Piper/VITS 和 Supertonic 本地发音模型。
//!
//! 使用项目已有的 sherpa-onnx；Piper 需要 tokens 与字典资源，Supertonic 需要完整模型包。

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use serde::Serialize;
use sherpa_onnx::{
    GenerationConfig, OfflineTts, OfflineTtsConfig, OfflineTtsModelConfig,
    OfflineTtsSupertonicModelConfig, OfflineTtsVitsModelConfig,
};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Serialize)]
pub(crate) struct LocalTtsVoice {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) lang: String,
}

struct PiperModel {
    voice: LocalTtsVoice,
    model: PathBuf,
    tokens: PathBuf,
    data_dir: Option<PathBuf>,
    lexicon: Option<PathBuf>,
    rule_fsts: Option<String>,
    noise_scale: f32,
    noise_scale_w: f32,
    length_scale: f32,
}

struct SupertonicModel {
    voice: LocalTtsVoice,
    directory: PathBuf,
    language: &'static str,
    speaker: i32,
}

enum LocalModel {
    Piper(PiperModel),
    Supertonic(SupertonicModel),
}

impl LocalModel {
    fn voice(&self) -> &LocalTtsVoice {
        match self {
            Self::Piper(model) => &model.voice,
            Self::Supertonic(model) => &model.voice,
        }
    }
}

static ENGINES: OnceLock<Mutex<HashMap<String, OfflineTts>>> = OnceLock::new();

fn root() -> PathBuf {
    crate::project_directory().join("assets").join("tts")
}

fn models_in_directory(dir: &Path, root: &Path) -> Vec<LocalModel> {
    if [
        "duration_predictor.int8.onnx",
        "text_encoder.int8.onnx",
        "vector_estimator.int8.onnx",
        "vocoder.int8.onnx",
        "tts.json",
        "unicode_indexer.bin",
        "voice.bin",
    ]
    .iter()
    .all(|file| dir.join(file).is_file())
    {
        let relative = dir
            .strip_prefix(root)
            .unwrap_or(dir)
            .to_string_lossy()
            .replace('\\', "/");
        return ["ko", "ja", "en"]
            .into_iter()
            .flat_map(|language| (0..10).map(move |speaker| (language, speaker)))
            .map(|(language, speaker)| {
                LocalModel::Supertonic(SupertonicModel {
                    voice: LocalTtsVoice {
                        id: format!("supertonic:{relative}:{language}:{speaker}"),
                        name: format!(
                            "Supertonic 3 · {} · {}",
                            language.to_uppercase(),
                            speaker + 1
                        ),
                        lang: match language {
                            "ko" => "ko-KR",
                            "ja" => "ja-JP",
                            _ => "en-US",
                        }
                        .into(),
                    },
                    directory: dir.to_path_buf(),
                    language,
                    speaker,
                })
            })
            .collect();
    }
    let tokens = dir.join("tokens.txt");
    let data_dir = dir.join("espeak-ng-data");
    let lexicon = dir.join("lexicon.txt");
    if !tokens.is_file() || (!data_dir.is_dir() && !lexicon.is_file()) {
        return Vec::new();
    }
    let rule_fsts = ["phone.fst", "date.fst", "number.fst"]
        .iter()
        .map(|name| dir.join(name))
        .filter(|path| path.is_file())
        .map(|path| path.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let mut models = Vec::new();
    for file in fs::read_dir(dir).into_iter().flatten().flatten() {
        let model = file.path();
        if !model.is_file() || model.extension().is_none_or(|ext| ext != "onnx") {
            continue;
        }
        let config_path = PathBuf::from(format!("{}.json", model.display()));
        let Ok(contents) = fs::read(config_path) else {
            continue;
        };
        let Ok(config) = serde_json::from_slice::<serde_json::Value>(&contents) else {
            continue;
        };
        let Some(lang) = config["language"]["code"]
            .as_str()
            .map(|value| value.replace('_', "-"))
        else {
            continue;
        };
        let Some(name) = model
            .file_stem()
            .map(|value| value.to_string_lossy().to_string())
        else {
            continue;
        };
        let Ok(relative) = model.strip_prefix(root) else {
            continue;
        };
        let relative = relative.to_string_lossy().replace('\\', "/");
        let inference = &config["inference"];
        models.push(LocalModel::Piper(PiperModel {
            voice: LocalTtsVoice {
                id: format!("piper:{relative}"),
                name: format!("{name} · Piper"),
                lang,
            },
            model,
            tokens: tokens.clone(),
            data_dir: data_dir.is_dir().then(|| data_dir.clone()),
            lexicon: lexicon.is_file().then(|| lexicon.clone()),
            rule_fsts: (!rule_fsts.is_empty()).then(|| rule_fsts.join(",")),
            noise_scale: inference["noise_scale"].as_f64().unwrap_or(0.667) as f32,
            noise_scale_w: inference["noise_w"].as_f64().unwrap_or(0.8) as f32,
            length_scale: inference["length_scale"].as_f64().unwrap_or(1.0) as f32,
        }));
    }
    models
}

fn collect_models(dir: &Path, root: &Path, depth: usize, models: &mut Vec<LocalModel>) {
    models.extend(models_in_directory(dir, root));
    if depth == 0 {
        return;
    }
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                collect_models(&entry.path(), root, depth - 1, models);
            }
        }
    }
}

fn models() -> Vec<LocalModel> {
    let root = root();
    let mut models = Vec::new();
    collect_models(&root, &root, 8, &mut models);
    models.sort_by(|left, right| left.voice().name.cmp(&right.voice().name));
    models
}

pub(crate) fn list_voices() -> Vec<LocalTtsVoice> {
    models()
        .into_iter()
        .map(|model| model.voice().clone())
        .collect()
}

pub(crate) fn synthesize(voice_id: &str, text: &str) -> Result<String> {
    let (samples, sample_rate) = synthesize_samples(voice_id, text)?;
    let wav = wav_bytes(&samples, sample_rate);
    Ok(format!(
        "data:audio/wav;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(wav)
    ))
}

pub(crate) fn synthesize_samples(voice_id: &str, text: &str) -> Result<(Vec<f32>, u32)> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > 500 || text.contains('\0') {
        bail!("发音文字必须为 1–500 个字符，且不能包含空字符");
    }
    let model = models()
        .into_iter()
        .find(|model| model.voice().id == voice_id)
        .context("本地语音模型已被移动，或缺少配套资源")?;
    let engines = ENGINES.get_or_init(|| Mutex::new(HashMap::new()));
    let mut engines = engines
        .lock()
        .map_err(|_| anyhow::anyhow!("Piper 播放状态不可用"))?;
    let engine_key = match &model {
        LocalModel::Piper(_) => voice_id.to_owned(),
        LocalModel::Supertonic(model) => format!("supertonic:{}", model.directory.display()),
    };
    if !engines.contains_key(&engine_key) {
        let model_config = match &model {
            LocalModel::Piper(model) => OfflineTtsModelConfig {
                vits: OfflineTtsVitsModelConfig {
                    model: Some(model.model.to_string_lossy().into_owned()),
                    tokens: Some(model.tokens.to_string_lossy().into_owned()),
                    data_dir: model
                        .data_dir
                        .as_ref()
                        .map(|path| path.to_string_lossy().into_owned()),
                    lexicon: model
                        .lexicon
                        .as_ref()
                        .map(|path| path.to_string_lossy().into_owned()),
                    noise_scale: model.noise_scale,
                    noise_scale_w: model.noise_scale_w,
                    length_scale: model.length_scale,
                    ..Default::default()
                },
                num_threads: 2,
                provider: Some("cpu".into()),
                ..Default::default()
            },
            LocalModel::Supertonic(model) => {
                let file =
                    |name: &str| Some(model.directory.join(name).to_string_lossy().into_owned());
                OfflineTtsModelConfig {
                    supertonic: OfflineTtsSupertonicModelConfig {
                        duration_predictor: file("duration_predictor.int8.onnx"),
                        text_encoder: file("text_encoder.int8.onnx"),
                        vector_estimator: file("vector_estimator.int8.onnx"),
                        vocoder: file("vocoder.int8.onnx"),
                        tts_json: file("tts.json"),
                        unicode_indexer: file("unicode_indexer.bin"),
                        voice_style: file("voice.bin"),
                    },
                    num_threads: 2,
                    provider: Some("cpu".into()),
                    ..Default::default()
                }
            }
        };
        let config = OfflineTtsConfig {
            model: model_config,
            rule_fsts: match &model {
                LocalModel::Piper(model) => model.rule_fsts.clone(),
                LocalModel::Supertonic(_) => None,
            },
            ..Default::default()
        };
        let engine = OfflineTts::create(&config).context("无法加载本地语音模型，请检查配套资源")?;
        engines.insert(engine_key.clone(), engine);
    }
    let generation = match &model {
        LocalModel::Piper(_) => GenerationConfig::default(),
        LocalModel::Supertonic(model) => GenerationConfig {
            sid: model.speaker,
            num_steps: 8,
            extra: Some(HashMap::from([(
                "lang".into(),
                serde_json::json!(model.language),
            )])),
            ..Default::default()
        },
    };
    let audio = engines
        .get(&engine_key)
        .expect("inserted above")
        .generate_with_config::<fn(&[f32], f32) -> bool>(text, &generation, None)
        .context("本地模型无法生成语音")?;
    if audio.samples().is_empty() || audio.sample_rate() <= 0 {
        bail!("本地模型返回了空音频");
    }
    Ok((audio.samples().to_vec(), audio.sample_rate() as u32))
}

fn wav_bytes(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let pcm_len = (samples.len() * 2) as u32;
    let mut wav = Vec::with_capacity(44 + pcm_len as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + pcm_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&pcm_len.to_le_bytes());
    for sample in samples {
        let pcm = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        wav.extend_from_slice(&pcm.to_le_bytes());
    }
    wav
}

#[cfg(test)]
mod tests {
    use super::{list_voices, synthesize};

    #[test]
    fn huayan_model_generates_wav_when_installed() {
        let Some(voice) = list_voices()
            .into_iter()
            .find(|voice| voice.name.contains("huayan"))
        else {
            return;
        };
        let audio = synthesize(&voice.id, "你好").expect("Piper should synthesize Chinese");
        assert!(audio.starts_with("data:audio/wav;base64,"));
        assert!(audio.len() > 1000);
    }

    #[test]
    fn installed_multilingual_voices_generate_wav() {
        let voices = list_voices();
        for (directory, needle, language, text) in [
            (
                "vits-piper-zh_CN-chaowen-medium",
                "chaowen",
                "zh-CN",
                "你好，很高兴认识你。",
            ),
            (
                "vits-piper-en_US-lessac-high",
                "lessac-high",
                "en-US",
                "Hello, nice to meet you.",
            ),
            (
                "sherpa-onnx-supertonic-3-tts-int8-2026-05-11",
                "Supertonic 3 · KO · 1",
                "ko-KR",
                "안녕하세요. 반갑습니다.",
            ),
            (
                "sherpa-onnx-supertonic-3-tts-int8-2026-05-11",
                "Supertonic 3 · JA · 1",
                "ja-JP",
                "こんにちは。よろしくお願いします。",
            ),
        ] {
            if !super::root().join(directory).is_dir() {
                continue;
            }
            let voice = voices
                .iter()
                .find(|voice| voice.name.contains(needle))
                .expect("installed voice should appear in the list");
            assert_eq!(voice.lang, language);
            let audio = synthesize(&voice.id, text).expect("installed voice should synthesize");
            assert!(audio.starts_with("data:audio/wav;base64,"));
            assert!(audio.len() > 1000);
        }
    }
}
