//! 将语音结果送入结果产生时的前台窗口。
//!
//! 优先通过临时剪贴板与真实 Ctrl+V 输入，以兼容不接收 VK_PACKET 的游戏和浏览器；
//! 随后恢复原剪贴板。无法安全备份剪贴板时退回 Unicode 模拟输入。

use anyhow::Result;

#[cfg(feature = "tsf-notepad-prototype")]
#[derive(Debug)]
struct TextServiceSendError {
    code: Option<i32>,
    detail: String,
}

#[cfg(feature = "tsf-notepad-prototype")]
impl std::fmt::Display for TextServiceSendError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "记事本 TSF 原型未接受文字（代码 {:?}）：{}",
            self.code, self.detail
        )
    }
}

#[cfg(feature = "tsf-notepad-prototype")]
impl std::error::Error for TextServiceSendError {}

pub(crate) fn input_error_trace_code(error: &anyhow::Error) -> &'static str {
    #[cfg(feature = "tsf-notepad-prototype")]
    if let Some(error) = error.downcast_ref::<TextServiceSendError>() {
        return match error.code {
            Some(3) => "tsf_no_foreground",
            Some(4) => "tsf_bridge_missing",
            Some(5) => "tsf_edit_rejected",
            Some(7) => "tsf_bridge_other_process",
            _ => "tsf_sender_error",
        };
    }
    let _ = error;
    "error"
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VoiceInputMethod {
    Paste,
    Unicode,
    ManualPaste,
    #[cfg(feature = "tsf-notepad-prototype")]
    TextService,
}

#[cfg(windows)]
#[derive(Clone, Copy)]
pub(crate) struct VoiceInputTarget {
    window: isize,
    process_id: u32,
}

#[cfg(not(windows))]
#[derive(Clone, Copy)]
pub(crate) struct VoiceInputTarget;

impl VoiceInputTarget {
    #[cfg(windows)]
    pub(crate) fn capture() -> Option<Self> {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetForegroundWindow, GetWindowThreadProcessId,
        };

        let window = unsafe { GetForegroundWindow() };
        if window.is_null() {
            return None;
        }
        let mut process_id = 0;
        unsafe { GetWindowThreadProcessId(window, &mut process_id) };
        if process_id == 0 || process_id == std::process::id() {
            return None;
        }
        Some(Self {
            window: window as isize,
            process_id,
        })
    }

    #[cfg(not(windows))]
    pub(crate) fn capture() -> Option<Self> {
        None
    }

    #[cfg(windows)]
    pub(crate) fn type_if_active(
        self,
        text: &str,
        aion2_manual_paste: bool,
    ) -> Result<Option<VoiceInputMethod>> {
        if text.is_empty() || !self.is_active() {
            return Ok(None);
        }
        #[cfg(feature = "tsf-notepad-prototype")]
        if self.process_executable_name_matches("notepad.exe") {
            return self.send_to_notepad_text_service(text);
        }
        if self.is_aion2() {
            if aion2_manual_paste {
                if !self.is_active() {
                    return Ok(None);
                }
                arboard::Clipboard::new()
                    .and_then(|mut clipboard| clipboard.set_text(text))
                    .map_err(|error| anyhow::anyhow!("无法复制 Aion2 语音结果：{error}"))?;
                return Ok(Some(VoiceInputMethod::ManualPaste));
            }
            anyhow::bail!("Aion2 尚未接入可用的文本服务；不会发送模拟 Ctrl+V");
        }
        if modifiers_pressed() {
            return Ok(None);
        }
        // OLE 剪贴板要求 STA；独立线程也避免与 eframe 的 COM 初始化方式冲突。
        let text = text.to_owned();
        std::thread::spawn(move || self.paste_or_type(&text))
            .join()
            .map_err(|_| anyhow::anyhow!("语音输入线程异常退出"))?
    }

    #[cfg(not(windows))]
    pub(crate) fn type_if_active(
        self,
        _text: &str,
        _aion2_manual_paste: bool,
    ) -> Result<Option<VoiceInputMethod>> {
        Ok(None)
    }
}

#[cfg(windows)]
impl VoiceInputTarget {
    fn is_aion2(self) -> bool {
        self.process_executable_name_matches("Aion2.exe")
    }

    fn process_executable_name_matches(self, expected: &str) -> bool {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        };

        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snapshot == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = unsafe { Process32FirstW(snapshot, &mut entry) } != 0;
        let mut matches = false;
        while found {
            if entry.th32ProcessID == self.process_id {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|character| *character == 0)
                    .unwrap_or(entry.szExeFile.len());
                matches = executable_name_matches(&entry.szExeFile[..len], expected);
                break;
            }
            found = unsafe { Process32NextW(snapshot, &mut entry) } != 0;
        }
        unsafe { CloseHandle(snapshot) };
        matches
    }

    #[cfg(feature = "tsf-notepad-prototype")]
    fn send_to_notepad_text_service(self, text: &str) -> Result<Option<VoiceInputMethod>> {
        let sender = crate::project_directory()
            .join("windows_text_service")
            .join("build")
            .join("to_words_tsf_send.exe");
        if !sender.exists() {
            anyhow::bail!("记事本 TSF 原型尚未构建：{}", sender.display());
        }
        if !self.is_active() {
            return Ok(None);
        }
        let output = std::process::Command::new(sender)
            .arg(text)
            .output()
            .map_err(|error| anyhow::anyhow!("无法启动记事本 TSF 原型：{error}"))?;
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr);
            return Err(TextServiceSendError {
                code: output.status.code(),
                detail: detail.trim().to_string(),
            }
            .into());
        }
        Ok(Some(VoiceInputMethod::TextService))
    }

    fn is_active(self) -> bool {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetForegroundWindow, GetWindowThreadProcessId,
        };
        let foreground = unsafe { GetForegroundWindow() };
        if foreground as isize != self.window {
            return false;
        }
        let mut process_id = 0;
        unsafe { GetWindowThreadProcessId(foreground, &mut process_id) };
        process_id == self.process_id
    }

    fn paste_or_type(self, text: &str) -> Result<Option<VoiceInputMethod>> {
        use windows::Win32::Foundation::{GetLastError, SetLastError, WIN32_ERROR};
        use windows::Win32::System::Com::{
            COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize,
        };
        use windows::Win32::System::DataExchange::{
            CountClipboardFormats, GetClipboardSequenceNumber,
        };
        use windows::Win32::System::Ole::{OleFlushClipboard, OleGetClipboard, OleSetClipboard};

        if !self.is_active() || modifiers_pressed() {
            return Ok(None);
        }

        let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if initialized.is_err() {
            return self.type_unicode(text);
        }
        struct ComGuard;
        impl Drop for ComGuard {
            fn drop(&mut self) {
                unsafe { CoUninitialize() };
            }
        }
        let _com = ComGuard;

        let mut clipboard = match arboard::Clipboard::new() {
            Ok(clipboard) => clipboard,
            Err(_) => return self.type_unicode(text),
        };
        unsafe { SetLastError(WIN32_ERROR(0)) };
        let format_count = unsafe { CountClipboardFormats() };
        if format_count == 0 && unsafe { GetLastError() }.0 != 0 {
            return self.type_unicode(text);
        }
        let was_empty = format_count == 0;
        let original_text = clipboard.get_text().ok();
        // 图片、文件等非文本内容无法用纯文本安全恢复，保持原剪贴板并退回直输。
        if !was_empty && original_text.is_none() {
            return self.type_unicode(text);
        }
        let original = unsafe { OleGetClipboard() }.ok();
        if !was_empty && original.is_none() {
            return self.type_unicode(text);
        }
        if !self.is_active() || modifiers_pressed() {
            return Ok(None);
        }
        if clipboard.set_text(text).is_err() {
            return self.type_unicode(text);
        }
        let pasted_sequence = unsafe { GetClipboardSequenceNumber() };
        let pasted = if self.is_active() && !modifiers_pressed() {
            self.send_paste_chord()
        } else {
            Ok(false)
        };

        // 让目标程序处理粘贴消息后再恢复。若用户期间主动复制了别的内容，则不覆盖它。
        std::thread::sleep(std::time::Duration::from_millis(180));
        if pasted_sequence != 0 && unsafe { GetClipboardSequenceNumber() } == pasted_sequence {
            let mut restored = was_empty && clipboard.clear().is_ok();
            if !restored && let Some(original) = &original {
                for _ in 0..3 {
                    if unsafe { OleSetClipboard(original) }.is_ok()
                        && unsafe { OleFlushClipboard() }.is_ok()
                    {
                        restored = true;
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(30));
                }
            }
            if !restored && let Some(original_text) = original_text {
                restored = clipboard.set_text(original_text).is_ok();
            }
            if !restored {
                anyhow::bail!("文字可能已粘贴，但恢复原剪贴板失败");
            }
        }
        pasted.map(|sent| sent.then_some(VoiceInputMethod::Paste))
    }

    fn type_unicode(self, text: &str) -> Result<Option<VoiceInputMethod>> {
        if !self.is_active() || modifiers_pressed() {
            return Ok(None);
        }
        send_inputs(&unicode_inputs(text))?;
        Ok(Some(VoiceInputMethod::Unicode))
    }

    fn send_paste_chord(self) -> Result<bool> {
        let keys = paste_inputs();
        if let Err(error) = send_inputs(&keys[..1]) {
            let _ = send_inputs(&keys[2..]);
            return Err(error);
        }
        // 游戏常按渲染帧查询键盘状态；同一 SendInput 批次内立即按下并释放可能被漏掉。
        std::thread::sleep(std::time::Duration::from_millis(30));
        let pressed = if self.is_active() && !modifiers_pressed_except_ctrl() {
            send_inputs(&keys[1..2]).map(|()| {
                std::thread::sleep(std::time::Duration::from_millis(70));
                true
            })
        } else {
            Ok(false)
        };
        // 所有路径都必须释放 Ctrl/V，避免游戏继续认为 Ctrl 被按住。
        send_inputs(&keys[2..])?;
        pressed
    }
}

#[cfg(all(test, windows))]
fn aion2_executable_name(name: &[u16]) -> bool {
    executable_name_matches(name, "Aion2.exe")
}

#[cfg(windows)]
fn executable_name_matches(name: &[u16], expected: &str) -> bool {
    String::from_utf16_lossy(name).eq_ignore_ascii_case(expected)
}

#[cfg(windows)]
fn modifiers_pressed() -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };
    [VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN]
        .iter()
        .any(|key| unsafe { GetAsyncKeyState(i32::from(*key)) } < 0)
}

#[cfg(windows)]
fn modifiers_pressed_except_ctrl() -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };
    [VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN]
        .iter()
        .any(|key| unsafe { GetAsyncKeyState(i32::from(*key)) } < 0)
}

#[cfg(windows)]
fn send_inputs(inputs: &[windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT]) -> Result<()> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{INPUT, SendInput};
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    if sent != inputs.len() as u32 {
        anyhow::bail!("Windows 未接受完整的模拟输入；目标程序可能以更高权限运行");
    }
    Ok(())
}

#[cfg(windows)]
fn paste_inputs() -> [windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT; 4] {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE,
        MAPVK_VK_TO_VSC, MapVirtualKeyW, VK_LCONTROL, VK_V,
    };
    let control_scan = unsafe { MapVirtualKeyW(u32::from(VK_LCONTROL), MAPVK_VK_TO_VSC) } as u16;
    let v_scan = unsafe { MapVirtualKeyW(u32::from(VK_V), MAPVK_VK_TO_VSC) } as u16;
    let key = |scan_code, flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: 0,
                wScan: scan_code,
                dwFlags: KEYEVENTF_SCANCODE | flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    [
        key(control_scan, 0),
        key(v_scan, 0),
        key(v_scan, KEYEVENTF_KEYUP),
        key(control_scan, KEYEVENTF_KEYUP),
    ]
}

#[cfg(windows)]
fn unicode_inputs(text: &str) -> Vec<windows_sys::Win32::UI::Input::KeyboardAndMouse::INPUT> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
    };

    let key = |unit, flags| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: 0,
                wScan: unit,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    text.encode_utf16()
        .flat_map(|unit| {
            [
                key(unit, KEYEVENTF_UNICODE),
                key(unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP),
            ]
        })
        .collect()
}

#[cfg(all(test, windows))]
mod tests {
    use super::{aion2_executable_name, paste_inputs, unicode_inputs};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{KEYEVENTF_KEYUP, KEYEVENTF_UNICODE};

    #[test]
    fn unicode_input_preserves_cjk_and_surrogate_pairs_without_clipboard() {
        let inputs = unicode_inputs("韩🙂");
        let units: Vec<u16> = inputs
            .chunks_exact(2)
            .map(|pair| {
                let down = unsafe { pair[0].Anonymous.ki };
                let up = unsafe { pair[1].Anonymous.ki };
                assert_eq!(down.wVk, 0);
                assert_eq!(down.dwFlags, KEYEVENTF_UNICODE);
                assert_eq!(up.wScan, down.wScan);
                assert_eq!(up.dwFlags, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP);
                down.wScan
            })
            .collect();
        assert_eq!(units, "韩🙂".encode_utf16().collect::<Vec<_>>());
    }

    #[test]
    fn paste_uses_real_control_v_key_events() {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE,
        };
        let keys: Vec<_> = paste_inputs()
            .iter()
            .map(|input| unsafe { input.Anonymous.ki })
            .collect();
        assert_eq!(keys[0].wVk, 0);
        assert_ne!(keys[0].wScan, 0);
        assert_eq!(keys[0].dwFlags, KEYEVENTF_SCANCODE);
        assert_eq!(keys[1].wVk, 0);
        assert_ne!(keys[1].wScan, 0);
        assert_eq!(keys[1].dwFlags, KEYEVENTF_SCANCODE);
        assert_eq!(keys[2].wScan, keys[1].wScan);
        assert_eq!(keys[2].dwFlags, KEYEVENTF_SCANCODE | KEYEVENTF_KEYUP);
        assert_eq!(keys[3].wScan, keys[0].wScan);
        assert_eq!(keys[3].dwFlags, KEYEVENTF_SCANCODE | KEYEVENTF_KEYUP);
    }

    #[test]
    fn aion2_compatibility_only_matches_the_game_executable() {
        assert!(aion2_executable_name(
            &"AION2.EXE".encode_utf16().collect::<Vec<_>>()
        ));
        assert!(!aion2_executable_name(
            &"Aion2Launcher.exe".encode_utf16().collect::<Vec<_>>()
        ));
    }
}
