//! Windows 全局鼠标按键监听，支持左、右、中键及 X1/X2 侧键。
//!
//! 低级鼠标钩子只观察按下/松开并立即转发事件；不拦截鼠标事件，也不在回调中
//! 执行业务逻辑。钩子安装在线程自有的 Windows 消息循环中。

use crate::domain::shortcut::{MouseButton, MouseModifiers};
#[cfg(not(target_os = "windows"))]
use anyhow::Result;
use std::time::Instant;

#[derive(Clone, Copy, Debug)]
pub(crate) struct MouseButtonEvent {
    pub(crate) button: MouseButton,
    pub(crate) pressed: bool,
    pub(crate) modifiers: MouseModifiers,
    pub(crate) occurred_at: Instant,
}

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::{MouseButton, MouseButtonEvent, MouseModifiers};
    use anyhow::{Context, Result};
    use std::cell::RefCell;
    use std::mem::zeroed;
    use std::ptr;
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::thread::{self, JoinHandle};
    use std::time::Instant;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetMessageW, MSG, MSLLHOOKSTRUCT, PM_NOREMOVE,
        PeekMessageW, PostThreadMessageW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx,
        WH_MOUSE_LL, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_QUIT,
        WM_RBUTTONDOWN, WM_RBUTTONUP, WM_XBUTTONDOWN, WM_XBUTTONUP, XBUTTON1, XBUTTON2,
    };

    thread_local! {
        static SENDER: RefCell<Option<Sender<MouseButtonEvent>>> = const { RefCell::new(None) };
    }

    pub(crate) struct MouseHotkeyListener {
        receiver: Receiver<MouseButtonEvent>,
        thread_id: u32,
        thread: Option<JoinHandle<()>>,
    }

    impl MouseHotkeyListener {
        pub(crate) fn new() -> Result<Self> {
            let (event_sender, receiver) = mpsc::channel();
            let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
            let thread = thread::Builder::new()
                .name("to_words_mouse_hotkeys".to_string())
                .spawn(move || {
                    SENDER.with(|slot| *slot.borrow_mut() = Some(event_sender));
                    let thread_id = unsafe { GetCurrentThreadId() };
                    let mut message: MSG = unsafe { zeroed() };
                    unsafe {
                        PeekMessageW(&mut message, ptr::null_mut(), 0, 0, PM_NOREMOVE);
                    }
                    let module = unsafe { GetModuleHandleW(ptr::null()) };
                    let hook =
                        unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), module, 0) };
                    if hook.is_null() {
                        let _ = ready_sender.send(Err(std::io::Error::last_os_error().to_string()));
                        return;
                    }
                    if ready_sender.send(Ok(thread_id)).is_err() {
                        unsafe { UnhookWindowsHookEx(hook) };
                        return;
                    }
                    while unsafe { GetMessageW(&mut message, ptr::null_mut(), 0, 0) } > 0 {
                        unsafe {
                            TranslateMessage(&message);
                            DispatchMessageW(&message);
                        }
                    }
                    unsafe { UnhookWindowsHookEx(hook) };
                    SENDER.with(|slot| *slot.borrow_mut() = None);
                })
                .context("无法创建鼠标快捷键监听线程")?;
            let thread_id = match ready_receiver.recv().context("鼠标快捷键监听线程未启动")?
            {
                Ok(thread_id) => thread_id,
                Err(error) => {
                    let _ = thread.join();
                    anyhow::bail!("无法安装全局鼠标监听：{error}");
                }
            };
            Ok(Self {
                receiver,
                thread_id,
                thread: Some(thread),
            })
        }

        pub(crate) fn try_recv(&self) -> Option<MouseButtonEvent> {
            self.receiver.try_recv().ok()
        }
    }

    impl Drop for MouseHotkeyListener {
        fn drop(&mut self) {
            unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    unsafe extern "system" fn mouse_hook(code: i32, message: usize, data: isize) -> isize {
        if code >= 0 {
            let info = unsafe { &*(data as *const MSLLHOOKSTRUCT) };
            if let Some((button, pressed)) = decode_button(message as u32, info.mouseData) {
                let event = MouseButtonEvent {
                    button,
                    pressed,
                    modifiers: current_modifiers(),
                    occurred_at: Instant::now(),
                };
                SENDER.with(|slot| {
                    if let Some(sender) = slot.borrow().as_ref() {
                        let _ = sender.send(event);
                    }
                });
            }
        }
        unsafe { CallNextHookEx(ptr::null_mut(), code, message, data) }
    }

    fn decode_button(message: u32, mouse_data: u32) -> Option<(MouseButton, bool)> {
        match message {
            WM_LBUTTONDOWN => Some((MouseButton::Left, true)),
            WM_LBUTTONUP => Some((MouseButton::Left, false)),
            WM_RBUTTONDOWN => Some((MouseButton::Right, true)),
            WM_RBUTTONUP => Some((MouseButton::Right, false)),
            WM_MBUTTONDOWN => Some((MouseButton::Middle, true)),
            WM_MBUTTONUP => Some((MouseButton::Middle, false)),
            WM_XBUTTONDOWN | WM_XBUTTONUP => {
                let button = match (mouse_data >> 16) as u16 {
                    XBUTTON1 => MouseButton::X1,
                    XBUTTON2 => MouseButton::X2,
                    _ => return None,
                };
                Some((button, message == WM_XBUTTONDOWN))
            }
            _ => None,
        }
    }

    fn current_modifiers() -> MouseModifiers {
        let pressed = |key: u16| unsafe { (GetAsyncKeyState(i32::from(key)) as u16 & 0x8000) != 0 };
        MouseModifiers::new(
            pressed(VK_CONTROL),
            pressed(VK_MENU),
            pressed(VK_SHIFT),
            pressed(VK_LWIN) || pressed(VK_RWIN),
        )
    }

    #[cfg(test)]
    mod tests {
        use super::{MouseButton, WM_XBUTTONDOWN, WM_XBUTTONUP, XBUTTON1, XBUTTON2, decode_button};

        #[test]
        fn side_buttons_are_distinguished_on_press_and_release() {
            assert_eq!(
                decode_button(WM_XBUTTONDOWN, u32::from(XBUTTON1) << 16),
                Some((MouseButton::X1, true))
            );
            assert_eq!(
                decode_button(WM_XBUTTONUP, u32::from(XBUTTON2) << 16),
                Some((MouseButton::X2, false))
            );
        }
    }
}

#[cfg(target_os = "windows")]
pub(crate) use windows_impl::MouseHotkeyListener;

#[cfg(not(target_os = "windows"))]
pub(crate) struct MouseHotkeyListener;

#[cfg(not(target_os = "windows"))]
impl MouseHotkeyListener {
    pub(crate) fn new() -> Result<Self> {
        Ok(Self)
    }

    pub(crate) fn try_recv(&self) -> Option<MouseButtonEvent> {
        None
    }
}
