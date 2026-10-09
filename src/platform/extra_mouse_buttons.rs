//! 补充监听驱动实际暴露的第 6～8 个鼠标按键。
//!
//! Windows 常规鼠标消息只包含 X1/X2；DirectInput 的 DIMOUSESTATE2 最多
//! 提供八个逻辑按钮。仅读取第 6～8 键，前五键继续使用低级鼠标钩子，
//! 避免重复触发。驱动若把不同物理键映射为同一个 X1/X2，则无法区分。

use crate::platform::mouse_hotkey::MouseButtonEvent;
#[cfg(target_os = "windows")]
mod windows_impl {
    use super::MouseButtonEvent;
    use crate::domain::shortcut::{MouseButton, MouseModifiers};
    use anyhow::{Context, Result};
    use std::ffi::c_void;
    use std::mem::size_of;
    use std::time::Instant;
    use windows::Win32::Devices::HumanInterfaceDevice::{
        DIDATAFORMAT, DIMOUSESTATE2, DISCL_BACKGROUND, DISCL_NONEXCLUSIVE, DirectInput8Create,
        GUID_SysMouse, IDirectInput8W, IDirectInputDevice8W,
    };
    use windows::Win32::Foundation::{HINSTANCE, HWND};
    use windows::core::Interface;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
    };

    #[link(name = "dinput8", kind = "static")]
    unsafe extern "C" {
        static c_dfDIMouse2: DIDATAFORMAT;
    }

    #[link(name = "dxguid", kind = "static")]
    unsafe extern "C" {}

    pub(crate) struct ExtraMouseButtons {
        _input: IDirectInput8W,
        device: IDirectInputDevice8W,
        previous: Option<[bool; 3]>,
    }

    impl ExtraMouseButtons {
        pub(crate) fn new(window: isize) -> Result<Self> {
            let module = unsafe { GetModuleHandleW(std::ptr::null()) };
            anyhow::ensure!(!module.is_null(), "无法取得程序模块句柄");
            let mut input: Option<IDirectInput8W> = None;
            unsafe {
                DirectInput8Create(
                    HINSTANCE(module),
                    0x0800,
                    &IDirectInput8W::IID,
                    &mut input as *mut _ as *mut *mut c_void,
                    None::<&windows::core::IUnknown>,
                )
            }
            .context("无法初始化 DirectInput")?;
            let input = input.context("DirectInput 未返回接口")?;
            let mut device = None;
            unsafe {
                input.CreateDevice(
                    &GUID_SysMouse,
                    &mut device,
                    None::<&windows::core::IUnknown>,
                )
            }
            .context("无法打开系统鼠标")?;
            let device = device.context("DirectInput 未返回鼠标设备")?;

            // 使用 Windows SDK 的标准格式；自制格式在少于八键的鼠标上可能失败。
            unsafe { device.SetDataFormat(&raw const c_dfDIMouse2 as *mut DIDATAFORMAT) }
                .context("无法设置八键鼠标数据格式")?;
            unsafe {
                device.SetCooperativeLevel(
                    HWND(window as *mut c_void),
                    DISCL_BACKGROUND | DISCL_NONEXCLUSIVE,
                )
            }
            .context("无法在后台读取鼠标扩展键")?;
            unsafe { device.Acquire() }.context("无法开始读取鼠标扩展键")?;
            Ok(Self {
                _input: input,
                device,
                previous: None,
            })
        }

        pub(crate) fn poll(&mut self) -> Vec<MouseButtonEvent> {
            let mut state = DIMOUSESTATE2::default();
            if unsafe {
                self.device.GetDeviceState(
                    size_of::<DIMOUSESTATE2>() as u32,
                    &mut state as *mut _ as *mut c_void,
                )
            }
            .is_err()
            {
                self.previous = None;
                let _ = unsafe { self.device.Acquire() };
                return Vec::new();
            }
            let current = [
                state.rgbButtons[5] & 0x80 != 0,
                state.rgbButtons[6] & 0x80 != 0,
                state.rgbButtons[7] & 0x80 != 0,
            ];
            let Some(previous) = self.previous.replace(current) else {
                return Vec::new();
            };
            let modifiers = current_modifiers();
            let occurred_at = Instant::now();
            [
                MouseButton::Extra6,
                MouseButton::Extra7,
                MouseButton::Extra8,
            ]
            .into_iter()
            .enumerate()
            .filter(|(index, _)| previous[*index] != current[*index])
            .map(|(index, button)| MouseButtonEvent {
                button,
                pressed: current[index],
                modifiers,
                occurred_at,
            })
            .collect()
        }

        pub(crate) fn reset(&mut self) {
            self.previous = None;
        }
    }

    impl Drop for ExtraMouseButtons {
        fn drop(&mut self) {
            let _ = unsafe { self.device.Unacquire() };
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
}

#[cfg(target_os = "windows")]
pub(crate) use windows_impl::ExtraMouseButtons;

#[cfg(not(target_os = "windows"))]
pub(crate) struct ExtraMouseButtons;

#[cfg(not(target_os = "windows"))]
impl ExtraMouseButtons {
    pub(crate) fn new(_window: isize) -> anyhow::Result<Self> {
        Ok(Self)
    }

    pub(crate) fn poll(&mut self) -> Vec<MouseButtonEvent> {
        Vec::new()
    }

    pub(crate) fn reset(&mut self) {}
}
