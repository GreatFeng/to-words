//! Low-latency, stateful subtitle hypotheses from a local sherpa-onnx model.
//! SenseVoice remains the source of the final text at each utterance boundary.

use crate::web_config::project_directory;
use anyhow::{Context, Result};
use sherpa_onnx::{OnlineRecognizer, OnlineRecognizerConfig, OnlineStream};
use std::path::PathBuf;

const MODEL_DIR: &str = "assets/voice/streaming/zipformer-small-zh-en";

pub(crate) struct StreamingAsr {
    recognizer: OnlineRecognizer,
    stream: OnlineStream,
    last_text: String,
}

impl StreamingAsr {
    pub(crate) fn load() -> Result<Option<Self>> {
        let root = project_directory().join(MODEL_DIR);
        let encoder = root.join("encoder-epoch-99-avg-1.int8.onnx");
        let decoder = root.join("decoder-epoch-99-avg-1.onnx");
        let joiner = root.join("joiner-epoch-99-avg-1.int8.onnx");
        let tokens = root.join("tokens.txt");
        if ![&encoder, &decoder, &joiner, &tokens]
            .iter()
            .all(|path| path.is_file())
        {
            return Ok(None);
        }
        let mut config = OnlineRecognizerConfig::default();
        config.model_config.transducer.encoder = Some(path_string(encoder));
        config.model_config.transducer.decoder = Some(path_string(decoder));
        config.model_config.transducer.joiner = Some(path_string(joiner));
        config.model_config.tokens = Some(path_string(tokens));
        config.model_config.num_threads = 2;
        config.decoding_method = Some("greedy_search".into());
        let recognizer = OnlineRecognizer::create(&config).context("无法加载流式字幕模型")?;
        let stream = recognizer.create_stream();
        Ok(Some(Self {
            recognizer,
            stream,
            last_text: String::new(),
        }))
    }

    pub(crate) fn reset(&mut self) {
        self.recognizer.reset(&self.stream);
        self.last_text.clear();
    }

    pub(crate) fn accept(&mut self, samples: &[i16]) -> Option<String> {
        let waveform: Vec<f32> = samples
            .iter()
            .map(|sample| *sample as f32 / 32768.0)
            .collect();
        self.stream.accept_waveform(16_000, &waveform);
        while self.recognizer.is_ready(&self.stream) {
            self.recognizer.decode(&self.stream);
        }
        let text = self
            .recognizer
            .get_result(&self.stream)?
            .text
            .trim()
            .to_owned();
        if text.is_empty() || text == self.last_text {
            return None;
        }
        self.last_text = text.clone();
        Some(text)
    }
}

fn path_string(path: PathBuf) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::StreamingAsr;

    #[test]
    #[ignore = "requires the optional model files and local speech sample"]
    fn streaming_model_decodes_local_recording() {
        let root = crate::web_config::project_directory();
        let sample = root.join("assets/voice/test_samples/test-voice2.wav");
        let converted = std::env::temp_dir().join(format!(
            "to_words_streaming_test_{}.wav",
            std::process::id()
        ));
        let status = std::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
            .arg(&sample)
            .args(["-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le"])
            .arg(&converted)
            .status()
            .expect("ffmpeg installed for test");
        assert!(status.success());
        let wave =
            sherpa_onnx::Wave::read(&converted.to_string_lossy()).expect("read converted sample");
        let asr = StreamingAsr::load().unwrap().expect("model installed");
        let mut latest = String::new();
        // The test sample is not guaranteed to be 16 kHz. The online API
        // resamples when given the source sample rate.
        for chunk in wave.samples().chunks(wave.sample_rate() as usize / 5) {
            asr.stream.accept_waveform(wave.sample_rate(), chunk);
            while asr.recognizer.is_ready(&asr.stream) {
                asr.recognizer.decode(&asr.stream);
            }
            if let Some(text) = asr
                .recognizer
                .get_result(&asr.stream)
                .map(|result| result.text)
            {
                if !text.is_empty() {
                    latest = text;
                }
            }
        }
        assert!(!latest.is_empty(), "streaming recognizer returned no text");
        println!("streaming result: {latest}");
        let _ = std::fs::remove_file(converted);
    }
}
