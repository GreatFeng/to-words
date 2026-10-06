//! 语音悬浮层的 Windows 原生窗口外观修正。
//!
//! egui 不绘制边框，但 Windows 11 仍可能给透明无装饰窗口添加系统边线。

#[cfg(windows)]
pub(crate) fn suppress_border() -> bool {
    use windows_sys::Win32::Graphics::Dwm::{
        DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DwmSetWindowAttribute,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowW, GWL_EXSTYLE, GetWindowLongPtrW, GetWindowThreadProcessId, SWP_FRAMECHANGED,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos,
        WS_EX_NOACTIVATE,
    };

    let title: Vec<u16> = "to_words 语音识别".encode_utf16().chain([0]).collect();
    let hwnd = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
    if hwnd.is_null() {
        return false;
    }
    let mut window_process_id = 0;
    unsafe { GetWindowThreadProcessId(hwnd, &mut window_process_id) };
    if window_process_id != std::process::id() {
        return false;
    }

    // 透明悬浮层仅展示文字；即使重新调整大小，也绝不能抢走游戏或浏览器输入焦点。
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
    if style & WS_EX_NOACTIVATE as isize == 0 {
        unsafe { SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_NOACTIVATE as isize) };
        unsafe {
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
            )
        };
    }

    let border_color = DWMWA_COLOR_NONE;
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR as u32,
            &border_color as *const u32 as *const _,
            std::mem::size_of_val(&border_color) as u32,
        ) >= 0
    }
}

#[cfg(not(windows))]
pub(crate) fn suppress_border() -> bool {
    false
}
