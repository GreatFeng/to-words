//! Windows 本地语音合成。返回 PCM，以便精确送往用户选定的虚拟音频播放端。

use anyhow::{Context, Result, bail};
use serde::Serialize;

#[derive(Clone, Serialize)]
pub(crate) struct WindowsTtsVoice {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) lang: String,
}

#[cfg(windows)]
pub(crate) fn list_voices() -> Result<Vec<WindowsTtsVoice>> {
    use windows::Media::SpeechSynthesis::SpeechSynthesizer;

    ensure_com()?;
    let all = SpeechSynthesizer::AllVoices()?;
    let mut result = Vec::new();
    for index in 0..all.Size()? {
        let voice = all.GetAt(index)?;
        result.push(WindowsTtsVoice {
            id: format!("winrt:{}", voice.Id()?),
            name: format!("{} · Windows", voice.DisplayName()?),
            lang: voice.Language()?.to_string(),
        });
    }
    result.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(result)
}

#[cfg(not(windows))]
pub(crate) fn list_voices() -> Result<Vec<WindowsTtsVoice>> {
    Ok(Vec::new())
}

#[cfg(windows)]
pub(crate) fn synthesize(voice_id: &str, text: &str) -> Result<(Vec<f32>, u32)> {
    use windows::Media::SpeechSynthesis::SpeechSynthesizer;
    use windows::Storage::Streams::DataReader;
    use windows::core::HSTRING;

    ensure_com()?;
    let all = SpeechSynthesizer::AllVoices()?;
    let selected_id = voice_id
        .strip_prefix("winrt:")
        .context("无效的 Windows 发音语音")?;
    let synthesizer = SpeechSynthesizer::new()?;
    let mut found = false;
    for index in 0..all.Size()? {
        let voice = all.GetAt(index)?;
        if voice.Id()?.to_string() == selected_id {
            synthesizer.SetVoice(&voice)?;
            found = true;
            break;
        }
    }
    if !found {
        bail!("所选 Windows 发音语音已不可用，请在设置中重新选择");
    }
    let stream = synthesizer
        .SynthesizeTextToStreamAsync(&HSTRING::from(text))?
        .get()?;
    let size = stream.Size()?;
    if size > 20_000_000 {
        bail!("合成的音频过长");
    }
    let input = stream.GetInputStreamAt(0)?;
    let reader = DataReader::CreateDataReader(&input)?;
    let loaded = reader.LoadAsync(size as u32)?.get()? as usize;
    let mut bytes = vec![0; loaded];
    reader.ReadBytes(&mut bytes)?;
    decode_wave(&bytes)
}

#[cfg(windows)]
struct ComGuard(bool);

#[cfg(windows)]
thread_local! {
    // WinRT 的语音 factory 会缓存 COM 对象；保持该线程的 COM apartment 到线程结束。
    static COM: std::result::Result<ComGuard, String> = ComGuard::new().map_err(|error| error.to_string());
}

#[cfg(windows)]
fn ensure_com() -> Result<()> {
    COM.with(|value| {
        value
            .as_ref()
            .map(|_| ())
            .map_err(|error| anyhow::anyhow!("{error}"))
    })
}

#[cfg(windows)]
impl ComGuard {
    fn new() -> Result<Self> {
        use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
        let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if result.0 == 0x80010106u32 as i32 {
            return Ok(Self(false));
        }
        result.ok().context("无法初始化 Windows 本地发音服务")?;
        Ok(Self(true))
    }
}

#[cfg(windows)]
impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.0 {
            unsafe { windows::Win32::System::Com::CoUninitialize() };
        }
    }
}

#[cfg(not(windows))]
pub(crate) fn synthesize(_voice_id: &str, _text: &str) -> Result<(Vec<f32>, u32)> {
    bail!("Windows 发音仅支持 Windows")
}

/// WinRT 的 SpeechSynthesisStream 是 RIFF/WAVE，取出并混合为单声道 float PCM。
fn decode_wave(bytes: &[u8]) -> Result<(Vec<f32>, u32)> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        bail!("Windows 发音返回了无法识别的音频格式");
    }
    let mut format = None;
    let mut data = None;
    let mut offset = 12;
    while offset + 8 <= bytes.len() {
        let len = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into()?) as usize;
        let start = offset + 8;
        let Some(end) = start.checked_add(len).filter(|end| *end <= bytes.len()) else {
            break;
        };
        match &bytes[offset..offset + 4] {
            b"fmt " if len >= 16 => format = Some(&bytes[start..end]),
            b"data" => data = Some(&bytes[start..end]),
            _ => {}
        }
        offset = end + (len & 1);
    }
    let fmt = format.context("Windows 发音缺少格式信息")?;
    let data = data.context("Windows 发音缺少音频数据")?;
    let mut kind = u16::from_le_bytes(fmt[0..2].try_into()?);
    let channels = u16::from_le_bytes(fmt[2..4].try_into()?) as usize;
    let sample_rate = u32::from_le_bytes(fmt[4..8].try_into()?);
    let block_align = u16::from_le_bytes(fmt[12..14].try_into()?) as usize;
    let bits = u16::from_le_bytes(fmt[14..16].try_into()?);
    if kind == 0xfffe && fmt.len() >= 40 {
        kind = u16::from_le_bytes(fmt[24..26].try_into()?);
    }
    let bytes_per_sample = (bits as usize).div_ceil(8);
    if channels == 0
        || sample_rate == 0
        || block_align < channels * bytes_per_sample
        || bytes_per_sample == 0
    {
        bail!("Windows 发音音频参数无效");
    }
    let mut samples = Vec::with_capacity(data.len() / block_align);
    for frame in data.chunks_exact(block_align) {
        let mut sum = 0.0;
        for channel in 0..channels {
            let pos = channel * bytes_per_sample;
            sum += match (kind, bits) {
                (1, 16) => i16::from_le_bytes(frame[pos..pos + 2].try_into()?) as f32 / 32768.0,
                (1, 24) => {
                    let raw = i32::from_le_bytes([
                        frame[pos],
                        frame[pos + 1],
                        frame[pos + 2],
                        if frame[pos + 2] & 0x80 != 0 { 0xff } else { 0 },
                    ]);
                    raw as f32 / 8_388_608.0
                }
                (1, 32) => {
                    i32::from_le_bytes(frame[pos..pos + 4].try_into()?) as f32 / 2_147_483_648.0
                }
                (3, 32) => f32::from_le_bytes(frame[pos..pos + 4].try_into()?),
                _ => bail!("Windows 发音返回了不支持的 PCM 格式 {kind}/{bits}"),
            };
        }
        samples.push((sum / channels as f32).clamp(-1.0, 1.0));
    }
    Ok((samples, sample_rate))
}

#[cfg(test)]
mod tests {
    use super::decode_wave;

    #[test]
    fn decodes_pcm16_mono_wave() {
        let mut wav = Vec::from(&b"RIFF\x26\x00\x00\x00WAVEfmt \x10\x00\x00\x00\x01\x00\x01\x00\x80\x3e\x00\x00\x00\x7d\x00\x00\x02\x00\x10\x00data\x02\x00\x00\x00\x00\x40"[..]);
        let (samples, rate) = decode_wave(&wav).unwrap();
        assert_eq!(rate, 16_000);
        assert!((samples[0] - 0.5).abs() < 0.001);
        wav[0] = 0;
        assert!(decode_wave(&wav).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn dedicated_audio_thread_lists_and_synthesizes() {
        std::thread::spawn(|| {
            let voices = super::list_voices().unwrap();
            let Some(voice) = voices.first() else { return };
            let (samples, sample_rate) = super::synthesize(&voice.id, "Hello").unwrap();
            assert!(!samples.is_empty());
            assert!(sample_rate >= 8_000);
        })
        .join()
        .unwrap();
    }
}
