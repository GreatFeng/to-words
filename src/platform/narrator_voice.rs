//! 读取当前用户在 Windows 讲述人中选用的语音名称。
//!
//! 这里只读取设置；讲述人的自然语音未必对第三方语音合成接口开放。

use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};

pub(crate) fn selected_voice() -> Option<String> {
    let subkey: Vec<u16> = "Software\\Microsoft\\Narrator\\NoRoam\0"
        .encode_utf16()
        .collect();
    let value: Vec<u16> = "SpeechVoice\0".encode_utf16().collect();
    let mut size = 0u32;
    // Windows 返回所需字节数；设置不存在或被拒绝时继续使用原有语音选择逻辑。
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS || size < 2 || size > 4096 {
        return None;
    }

    let mut buffer = vec![0u16; (size as usize).div_ceil(2)];
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let end = buffer
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(buffer.len());
    let name = String::from_utf16_lossy(&buffer[..end]);
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}
