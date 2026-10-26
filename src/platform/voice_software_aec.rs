//! Software echo cancellation using WASAPI playback loopback and Sonora AEC3.
//! Unlike the Windows communications effect, this works without microphone AEC support.

use anyhow::{Context, Result, bail};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use crate::platform::voice::{self, SAMPLE_RATE};
use crate::platform::voice_metrics::VoiceTrace;

const FRAME_SAMPLES: usize = SAMPLE_RATE / 100;
const MAX_RENDER_QUEUE: usize = SAMPLE_RATE / 5;

#[cfg(windows)]
pub(crate) fn record(
    stop: Arc<AtomicBool>,
    sender: Sender<Vec<i16>>,
    keep_input: bool,
    trace: VoiceTrace,
) -> Result<()> {
    use sonora::config::{EchoCanceller, NoiseSuppression, NoiseSuppressionLevel};
    use sonora::{AudioProcessing, Config, StreamConfig};

    let config = StreamConfig::new(SAMPLE_RATE as u32, 1);
    let mut processor = AudioProcessing::builder()
        .config(Config {
            echo_canceller: Some(EchoCanceller::default()),
            // Residual speaker audio after AEC can otherwise be recognized as
            // another utterance, especially around the end of a TTS clip.
            noise_suppression: Some(NoiseSuppression {
                level: NoiseSuppressionLevel::Low,
                ..Default::default()
            }),
            ..Default::default()
        })
        .capture_config(config)
        .render_config(config)
        .build();
    processor
        .set_stream_delay_ms(50)
        .map_err(|error| anyhow::anyhow!("软件回声消除延迟设置失败：{error}"))?;

    let (render_tx, render_rx) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::channel();
    let render_stop = Arc::clone(&stop);
    let render_worker = std::thread::spawn(move || loopback(render_stop, render_tx, ready_tx, ""));
    let readiness = ready_rx
        .recv_timeout(Duration::from_secs(4))
        .context("无法启动系统播放声音回采");
    if let Err(error) = readiness.and_then(|result| result) {
        stop.store(true, Ordering::Release);
        let _ = render_worker.join();
        return Err(error);
    }

    let (mic_tx, mic_rx) = mpsc::channel();
    let mic_stop = Arc::clone(&stop);
    let mic_trace = trace.clone();
    let mic_worker = std::thread::spawn(move || {
        voice::record_microphone(mic_stop, mic_tx, keep_input, mic_trace)
    });
    trace.record(0, "software_aec_ready", None, "ok");

    let mut render_queue = VecDeque::<f32>::new();
    let mut capture_queue = VecDeque::<f32>::new();
    let mut render_frame = [0.0_f32; FRAME_SAMPLES];
    let mut capture_frame = [0.0_f32; FRAME_SAMPLES];
    let mut output_frame = [0.0_f32; FRAME_SAMPLES];
    let mut ignored_render = [0.0_f32; FRAME_SAMPLES];
    let outcome = (|| -> Result<()> {
        loop {
            while let Ok(chunk) = render_rx.try_recv() {
                render_queue.extend(chunk.into_iter().map(pcm_to_float));
            }
            while render_queue.len() > MAX_RENDER_QUEUE {
                render_queue.pop_front();
            }
            if render_worker.is_finished() && !stop.load(Ordering::Acquire) {
                bail!("系统播放声音回采意外停止");
            }
            match mic_rx.recv_timeout(Duration::from_millis(40)) {
                Ok(chunk) => {
                    capture_queue.extend(chunk.into_iter().map(pcm_to_float));
                    let mut processed = Vec::new();
                    while capture_queue.len() >= FRAME_SAMPLES {
                        for sample in &mut capture_frame {
                            *sample = capture_queue.pop_front().unwrap_or_default();
                        }
                        for sample in &mut render_frame {
                            *sample = render_queue.pop_front().unwrap_or_default();
                        }
                        processor
                            .process_render_f32(&[&render_frame], &mut [&mut ignored_render])
                            .map_err(|error| anyhow::anyhow!("播放声分析失败：{error}"))?;
                        processor
                            .process_capture_f32(&[&capture_frame], &mut [&mut output_frame])
                            .map_err(|error| anyhow::anyhow!("软件回声消除失败：{error}"))?;
                        processed.extend(output_frame.iter().copied().map(float_to_pcm));
                    }
                    if !processed.is_empty() && sender.send(processed).is_err() {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) if mic_worker.is_finished() => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
        Ok(())
    })();
    stop.store(true, Ordering::Release);
    let microphone = mic_worker
        .join()
        .map_err(|_| anyhow::anyhow!("麦克风线程意外退出"))?;
    let playback = render_worker
        .join()
        .map_err(|_| anyhow::anyhow!("播放回采线程意外退出"))?;
    outcome.and(microphone).and(playback)
}

fn pcm_to_float(sample: i16) -> f32 {
    sample as f32 / 32768.0
}
fn float_to_pcm(sample: f32) -> i16 {
    (sample.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

/// Capture the default playback endpoint without opening a microphone.
#[cfg(windows)]
pub(crate) fn record_playback(
    stop: Arc<AtomicBool>,
    sender: Sender<Vec<i16>>,
    device_id: &str,
) -> Result<()> {
    let (ready, _receiver) = mpsc::channel();
    loopback(stop, sender, ready, device_id)
}

#[cfg(not(windows))]
pub(crate) fn record_playback(
    _stop: Arc<AtomicBool>,
    _sender: Sender<Vec<i16>>,
    _device_id: &str,
) -> Result<()> {
    bail!("系统声音监听仅支持 Windows")
}

#[cfg(windows)]
fn loopback(
    stop: Arc<AtomicBool>,
    sender: Sender<Vec<i16>>,
    ready: Sender<Result<()>>,
    device_id: &str,
) -> Result<()> {
    use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat, initialize_mta};

    let setup = (|| {
        initialize_mta()
            .ok()
            .context("无法初始化系统声音回采线程")?;
        let devices = DeviceEnumerator::new()?;
        let device = if device_id.is_empty() {
            devices
                .get_default_device(&Direction::Render)
                .context("没有默认播放设备")?
        } else {
            devices
                .get_device(device_id)
                .context("所选的系统声音播放设备不可用")?
        };
        let mut client = device.get_iaudioclient()?;
        let (_, period) = client.get_device_period()?;
        let format = WaveFormat::new(16, 16, &SampleType::Int, SAMPLE_RATE, 1, None);
        client
            .initialize_client(
                &format,
                &Direction::Capture,
                &StreamMode::EventsShared {
                    autoconvert: true,
                    buffer_duration_hns: period,
                },
            )
            .context("无法以 16 kHz 单声道回采系统播放声音")?;
        let event = client.set_get_eventhandle()?;
        let capture = client.get_audiocaptureclient()?;
        let buffer = vec![0_u8; client.get_buffer_size()? as usize * 4];
        client.start_stream()?;
        Ok::<_, anyhow::Error>((client, event, capture, buffer))
    })();
    let (client, event, capture, mut buffer) = match setup {
        Ok(value) => value,
        Err(error) => {
            let _ = ready.send(Err(anyhow::anyhow!("{error:#}")));
            return Err(error);
        }
    };
    let _ = ready.send(Ok(()));
    let outcome = (|| -> Result<()> {
        while !stop.load(Ordering::Acquire) {
            match event.wait_for_event(200) {
                Ok(()) | Err(wasapi::WasapiError::EventTimeout) => {}
                Err(error) => return Err(error).context("等待系统播放声音失败"),
            }
            while capture
                .get_next_packet_size()?
                .is_some_and(|frames| frames > 0)
            {
                let (frames, _) = capture.read_from_device(&mut buffer)?;
                if frames == 0 {
                    break;
                }
                let samples = buffer[..frames as usize * 2]
                    .chunks_exact(2)
                    .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
                    .collect();
                if sender.send(samples).is_err() {
                    break;
                }
            }
        }
        Ok(())
    })();
    let _ = client.stop_stream();
    outcome
}

#[cfg(not(windows))]
pub(crate) fn record(
    _stop: Arc<AtomicBool>,
    _sender: Sender<Vec<i16>>,
    _keep_input: bool,
    _trace: VoiceTrace,
) -> Result<()> {
    bail!("软件回声消除仅支持 Windows")
}

#[cfg(test)]
mod tests {
    use super::{FRAME_SAMPLES, float_to_pcm, pcm_to_float};

    #[test]
    fn pcm_conversion_is_bounded_and_frame_is_10ms() {
        assert_eq!(FRAME_SAMPLES, 160);
        assert_eq!(float_to_pcm(2.0), 32767);
        assert_eq!(float_to_pcm(-2.0), -32767);
        assert!((pcm_to_float(16384) - 0.5).abs() < 0.0001);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires a Windows default playback device"]
    fn default_playback_loopback_opens() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::mpsc;
        use std::time::Duration;

        let stop = Arc::new(AtomicBool::new(false));
        let (sender, _receiver) = mpsc::channel();
        let worker_stop = Arc::clone(&stop);
        let worker = std::thread::spawn(move || super::record_playback(worker_stop, sender, ""));
        std::thread::sleep(Duration::from_millis(500));
        stop.store(true, Ordering::Release);
        worker
            .join()
            .unwrap()
            .expect("default playback loopback should open");
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "plays the checked-in voice sample through the default output device"]
    fn default_playback_loopback_receives_samples() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::mpsc;

        let stop = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let worker_stop = Arc::clone(&stop);
        let worker = std::thread::spawn(move || super::record_playback(worker_stop, sender, ""));
        std::thread::sleep(std::time::Duration::from_millis(500));
        let sample = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("assets/voice/test_samples/test-voice2.wav");
        let output = std::process::Command::new("powershell")
            .arg("-NoProfile")
            .arg("-Command")
            .arg("(New-Object System.Media.SoundPlayer $env:TO_WORDS_TEST_WAV).PlaySync()")
            .env("TO_WORDS_TEST_WAV", sample)
            .output()
            .expect("playback command should start");
        std::thread::sleep(std::time::Duration::from_millis(800));
        stop.store(true, Ordering::Release);
        worker
            .join()
            .unwrap()
            .expect("loopback capture should finish");
        assert!(
            output.status.success(),
            "sample playback failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let mut samples = 0_usize;
        let mut nonzero = 0_usize;
        let mut gate = crate::platform::voice::SpeechGate::new_playback(0.6);
        let mut completed = false;
        for chunk in receiver.try_iter() {
            samples += chunk.len();
            nonzero += chunk.iter().filter(|value| **value != 0).count();
            let update = gate.push(&chunk);
            completed |= update.completed.is_some();
        }
        assert!(samples > 0, "loopback captured no audio packets");
        assert!(nonzero > 0, "loopback captured only silence");
        assert!(
            completed,
            "playback gate did not flush after playback stopped"
        );
    }
}
