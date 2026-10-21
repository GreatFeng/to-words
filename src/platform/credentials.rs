//! Base64 编码的 DeepSeek API Key 解码与本机凭据存储。
//!
//! Base64 仅是编码，并不保护密钥；Windows 使用当前用户的凭据管理器保存解码后的密钥。

use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};

fn decode_key(encoded: &str) -> Result<String> {
    let bytes = STANDARD
        .decode(encoded.trim())
        .context("API Key 不是有效的 Base64 字符串")?;
    let key = String::from_utf8(bytes).context("Base64 解码后的 API Key 不是 UTF-8 文本")?;
    ensure!(!key.is_empty(), "Base64 解码后的 API Key 不能为空");
    ensure!(
        !key.chars().any(char::is_whitespace),
        "Base64 解码后的 API Key 不能包含空白字符"
    );
    Ok(key)
}

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
pub(crate) fn save_key(encoded_key: &str) -> Result<()> {
    use windows::{
        Win32::Security::Credentials::{
            CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredWriteW,
        },
        core::PWSTR,
    };
    let key = decode_key(encoded_key)?;
    let mut bytes = key.into_bytes();
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

#[cfg(test)]
mod tests {
    use super::decode_key;

    #[test]
    fn decodes_base64_api_key() {
        assert_eq!(decode_key("  c2stZXhhbXBsZQ==  ").unwrap(), "sk-example");
    }

    #[test]
    fn rejects_plaintext_and_invalid_decoded_keys() {
        assert!(decode_key("sk-example").is_err());
        assert!(decode_key("!!!").is_err());
        assert!(decode_key("IA==").is_err());
        assert!(decode_key("c2sgZXhhbXBsZQ==").is_err());
    }
}
