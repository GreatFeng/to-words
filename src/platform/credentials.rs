//! DeepSeek API Key 的本机凭据存储。
//!
//! Windows 使用当前用户的凭据管理器，普通 JSON 配置和词库文件不保存密钥。

use anyhow::Result;

#[cfg(target_os = "windows")]
const TARGET: windows::core::PCWSTR = windows::core::w!("to_words/deepseek_api_key");

#[cfg(target_os = "windows")]
pub(crate) fn load_key() -> Result<Option<String>> {
    use windows::Win32::Security::Credentials::{
        CRED_TYPE_GENERIC, CREDENTIALW, CredFree, CredReadW,
    };
    let mut pointer: *mut CREDENTIALW = std::ptr::null_mut();
    if let Err(error) = unsafe { CredReadW(TARGET, CRED_TYPE_GENERIC, None, &mut pointer) } {
        if error.code().0 as u32 == 0x8007_0490 {
            return Ok(None);
        }
        return Err(error.into());
    }
    let credential = unsafe { &*pointer };
    let bytes = if credential.CredentialBlobSize == 0 {
        &[][..]
    } else {
        unsafe {
            std::slice::from_raw_parts(
                credential.CredentialBlob,
                credential.CredentialBlobSize as usize,
            )
        }
    };
    let result = String::from_utf8(bytes.to_vec()).map(Some);
    unsafe { CredFree(pointer.cast()) };
    Ok(result?)
}

#[cfg(target_os = "windows")]
pub(crate) fn save_key(key: &str) -> Result<()> {
    use windows::{
        Win32::Security::Credentials::{
            CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredWriteW,
        },
        core::PWSTR,
    };
    let mut bytes = key.as_bytes().to_vec();
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: PWSTR(TARGET.as_ptr() as *mut u16),
        UserName: PWSTR(windows::core::w!("to_words").as_ptr() as *mut u16),
        CredentialBlob: bytes.as_mut_ptr(),
        CredentialBlobSize: bytes.len() as u32,
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        ..Default::default()
    };
    unsafe { CredWriteW(&credential, 0) }?;
    Ok(())
}

#[cfg(target_os = "windows")]
pub(crate) fn delete_key() -> Result<()> {
    use windows::Win32::Security::Credentials::{CRED_TYPE_GENERIC, CredDeleteW};
    unsafe { CredDeleteW(TARGET, CRED_TYPE_GENERIC, None) }?;
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn load_key() -> Result<Option<String>> {
    Ok(std::env::var("DEEPSEEK_API_KEY").ok())
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn save_key(_key: &str) -> Result<()> {
    anyhow::bail!("当前系统暂不支持保存 API Key，请使用 DEEPSEEK_API_KEY 环境变量")
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn delete_key() -> Result<()> {
    anyhow::bail!("当前系统暂不支持删除 API Key")
}
