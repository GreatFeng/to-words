//! Windows communications capture with the system's acoustic echo cancellation.
//! This removes audio from the selected render endpoint before speech detection.

use anyhow::{Context, Result};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use crate::platform::voice_metrics::VoiceTrace;
use crate::platform::voice_software_aec;

#[cfg(windows)]
pub(crate) fn record(
    stop: Arc<AtomicBool>,
    sender: Sender<Vec<i16>>,
    keep_input: bool,
    trace: VoiceTrace,
) -> Result<()> {
    match record_hardware(Arc::clone(&stop), sender.clone(), keep_input, trace.clone()) {
        Ok(()) => Ok(()),
        Err(hardware_error) if !stop.load(Ordering::Acquire) => {
            trace.record(0, "hardware_aec", None, "software_fallback");
            voice_software_aec::record(stop, sender, keep_input, trace).with_context(|| {
                format!("硬件回声消除不可用（{hardware_error:#}），软件消除也无法启动")
            })
        }
        Err(error) => Err(error),
    }
}

#[cfg(windows)]
fn record_hardware(
    stop: Arc<AtomicBool>,
    sender: Sender<Vec<i16>>,
    keep_input: bool,
    trace: VoiceTrace,
) -> Result<()> {
    use wasapi::{
        AudioClientProperties, DeviceEnumerator, Direction, Role, SampleType, StreamCategory,
        StreamMode, WaveFormat, initialize_mta,
    };

    initialize_mta()
        .ok()
        .context("无法初始化 Windows 音频线程")?;
    let devices = DeviceEnumerator::new().context("无法列举音频设备")?;
    let microphone = devices
        .get_default_device_for_role(&Direction::Capture, &Role::Communications)
        .context("没有默认通信麦克风")?;
    let mut client = microphone
        .get_iaudioclient()
        .context("无法打开通信麦克风")?;
    client
        .set_properties(AudioClientProperties::new().set_category(StreamCategory::Communications))
        .context("无法设置通信音频流")?;
    let format = WaveFormat::new(16, 16, &SampleType::Int, 16_000, 1, None);
    let (_, minimum_period) = client.get_device_period()?;
    client
        .initialize_client(
            &format,
            &Direction::Capture,
            &StreamMode::EventsShared {
                autoconvert: true,
                buffer_duration_hns: minimum_period,
            },
        )
        .context("无法启动回声消除录音格式")?;
    if !matches!(client.is_aec_supported(), Ok(true)) {
        anyhow::bail!("当前通信麦克风不支持硬件 AEC");
    }
    let output = devices
        .get_default_device(&Direction::Render)
        .context("没有默认播放设备")?;
    client
        .get_aec_control()?
        .set_echo_cancellation_render_endpoint(Some(output.get_id()?))
        .context("无法绑定播放设备作为回声消除参考")?;
    let event = client.set_get_eventhandle()?;
    let capture = client.get_audiocaptureclient()?;
    let mut buffer = vec![0_u8; client.get_buffer_size()? as usize * 4];
    client.start_stream().context("无法开始回声消除录音")?;
    trace.record(0, "microphone_ready", None, "aec");
    let started = Instant::now();
    let outcome = (|| -> Result<()> {
        while !stop.load(Ordering::Acquire) {
            if !keep_input && started.elapsed() >= Duration::from_secs(90) {
                stop.store(true, Ordering::Release);
                break;
            }
            match event.wait_for_event(200) {
                Ok(()) | Err(wasapi::WasapiError::EventTimeout) => {}
                Err(error) => return Err(error).context("等待麦克风数据失败"),
            }
            while capture
                .get_next_packet_size()?
                .is_some_and(|count| count > 0)
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
    anyhow::bail!("回声消除仅支持 Windows")
}
