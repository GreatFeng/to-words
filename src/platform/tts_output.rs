//! 将本地合成的语音送到用户指定的 Windows 播放端（如 VB-CABLE Input）。
//! 聊天软件需将对应的录音端（如 VB-CABLE Output）选作麦克风。

use anyhow::{Context, Result, bail};
use serde::Serialize;

#[derive(Clone, Serialize)]
pub(crate) struct AudioOutputDevice {
    pub(crate) id: String,
    pub(crate) name: String,
}

#[cfg(windows)]
pub(crate) fn list_devices() -> Result<Vec<AudioOutputDevice>> {
    use wasapi::{DeviceEnumerator, Direction};

    let _com = ComGuard::new()?;
    let enumerator = DeviceEnumerator::new().context("无法枚举 Windows 音频设备")?;
    let collection = enumerator
        .get_device_collection(&Direction::Render)
        .context("无法读取音频播放设备")?;
    let mut devices = Vec::new();
    for index in 0..collection.get_nbr_devices()? {
        let device = collection.get_device_at_index(index)?;
        devices.push(AudioOutputDevice {
            id: device.get_id()?,
            name: device.get_friendlyname()?,
        });
    }
    devices.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(devices)
}

#[cfg(not(windows))]
pub(crate) fn list_devices() -> Result<Vec<AudioOutputDevice>> {
    Ok(Vec::new())
}

#[cfg(windows)]
struct ComGuard(bool);

#[cfg(windows)]
impl ComGuard {
    fn new() -> Result<Self> {
        let result = wasapi::initialize_mta();
        if result.0 == 0x80010106u32 as i32 {
            // Tauri WebView 线程可能已经是 STA；继续使用已有的 COM apartment。
            return Ok(Self(false));
        }
        result.ok().context("无法初始化 Windows 音频服务")?;
        Ok(Self(true))
    }
}

#[cfg(windows)]
impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 {
            wasapi::deinitialize();
        }
    }
}

#[cfg(windows)]
pub(crate) fn play_samples(
    device_id: &str,
    samples: &[f32],
    sample_rate: u32,
    cancelled: impl Fn() -> bool,
) -> Result<()> {
    use std::time::{Duration, Instant};
    use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};

    if cancelled() {
        return Ok(());
    }
    if device_id.is_empty() || samples.is_empty() || sample_rate == 0 {
        bail!("未选择输出设备，或发音音频为空");
    }
    let _com = ComGuard::new()?;
    let enumerator = DeviceEnumerator::new()?;
    let device = enumerator
        .get_device(device_id)
        .context("所选语音输出设备已断开，请在设置中重新选择")?;
    let mut client = device.get_iaudioclient()?;
    // 交给 Windows 音频引擎将模型采样率转换为虚拟设备的混音采样率。
    let format = WaveFormat::new(32, 32, &SampleType::Float, sample_rate as usize, 2, None);
    client.initialize_client(
        &format,
        &Direction::Render,
        &StreamMode::PollingShared {
            autoconvert: true,
            buffer_duration_hns: 1_000_000,
        },
    )?;
    let render = client.get_audiorenderclient()?;
    let mut position = 0;
    let deadline =
        Instant::now() + Duration::from_secs_f64(samples.len() as f64 / sample_rate as f64 + 5.0);
    let write_frames = |position: usize, frames: usize| -> Result<()> {
        let mut bytes = Vec::with_capacity(frames * 8);
        for sample in &samples[position..position + frames] {
            let value = sample.clamp(-1.0, 1.0).to_le_bytes();
            bytes.extend_from_slice(&value);
            bytes.extend_from_slice(&value);
        }
        render.write_to_device(frames, &bytes, None)?;
        Ok(())
    };
    let primed = (client.get_available_space_in_frames()? as usize).min(samples.len());
    write_frames(0, primed)?;
    position += primed;
    if cancelled() {
        return Ok(());
    }
    client.start_stream()?;
    while position < samples.len() {
        if cancelled() {
            client.stop_stream()?;
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("音频设备播放超时，请检查虚拟音频驱动");
        }
        let available = client.get_available_space_in_frames()? as usize;
        let frames = available.min(samples.len() - position);
        if frames == 0 {
            std::thread::sleep(Duration::from_millis(8));
            continue;
        }
        write_frames(position, frames)?;
        position += frames;
    }
    // 缓冲区仍在播放，先等它排空，避免 AudioClient 释放时截断结尾。
    while client.get_current_padding()? > 0 {
        if cancelled() {
            client.stop_stream()?;
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("音频设备播放超时，请检查虚拟音频驱动");
        }
        std::thread::sleep(Duration::from_millis(8));
    }
    client.stop_stream()?;
    Ok(())
}

#[cfg(not(windows))]
pub(crate) fn play_samples(
    _device_id: &str,
    _samples: &[f32],
    _sample_rate: u32,
    _cancelled: impl Fn() -> bool,
) -> Result<()> {
    bail!("自动麦克风发音仅支持 Windows")
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn audio_outputs_can_be_listed() {
        let devices = super::list_devices().unwrap();
        for device in &devices {
            println!("{}", device.name);
        }
    }
}
