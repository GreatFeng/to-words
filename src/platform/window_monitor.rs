//! 获取唤起设置窗口的显示器工作区，并将窗口居中于该显示器。

#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::POINT,
    Graphics::Gdi::{
        GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL, MONITORINFO,
        MonitorFromPoint, MonitorFromWindow,
    },
    UI::WindowsAndMessaging::{GetCursorPos, GetForegroundWindow},
};

#[derive(Clone, Copy)]
pub(crate) enum SettingsOrigin {
    Keyboard,
    Mouse,
}

#[cfg(windows)]
pub(crate) fn settings_position(
    origin: SettingsOrigin,
    window_size: (u32, u32),
) -> Option<(i32, i32)> {
    unsafe {
        let foreground = GetForegroundWindow();
        let foreground_monitor = if foreground.is_null() {
            std::ptr::null_mut()
        } else {
            MonitorFromWindow(foreground, MONITOR_DEFAULTTONULL)
        };
        let mut cursor = POINT { x: 0, y: 0 };
        let cursor_monitor = if GetCursorPos(&mut cursor) != 0 {
            MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST)
        } else {
            std::ptr::null_mut()
        };
        let monitor = match origin {
            SettingsOrigin::Keyboard if !foreground_monitor.is_null() => foreground_monitor,
            SettingsOrigin::Mouse if !cursor_monitor.is_null() => cursor_monitor,
            _ if !foreground_monitor.is_null() => foreground_monitor,
            _ => cursor_monitor,
        };
        if monitor.is_null() {
            return None;
        }
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return None;
        }
        let work = info.rcWork;
        Some(center_in_work_area(
            (work.left, work.top, work.right, work.bottom),
            window_size,
        ))
    }
}

/// Place subtitles over the work area of the active application, leaving room
/// for the taskbar and for controls at the bottom of a video.
#[cfg(windows)]
pub(crate) fn subtitle_bounds(desired_width: u32, height: u32) -> Option<(i32, i32, u32)> {
    unsafe {
        let foreground = GetForegroundWindow();
        let monitor = if !foreground.is_null() {
            MonitorFromWindow(foreground, MONITOR_DEFAULTTONEAREST)
        } else {
            let mut cursor = POINT { x: 0, y: 0 };
            if GetCursorPos(&mut cursor) == 0 {
                return None;
            }
            MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST)
        };
        if monitor.is_null() {
            return None;
        }
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return None;
        }
        let work = info.rcWork;
        let width = desired_width.min((work.right - work.left).max(1) as u32);
        let x = work.left + ((work.right - work.left - width as i32) / 2).max(0);
        let y = (work.bottom - height as i32 - 80).max(work.top);
        Some((x, y, width))
    }
}

#[cfg(not(windows))]
pub(crate) fn subtitle_bounds(_desired_width: u32, _height: u32) -> Option<(i32, i32, u32)> {
    None
}

#[cfg(not(windows))]
pub(crate) fn settings_position(
    _origin: SettingsOrigin,
    _window_size: (u32, u32),
) -> Option<(i32, i32)> {
    None
}

fn center_in_work_area(work: (i32, i32, i32, i32), size: (u32, u32)) -> (i32, i32) {
    let (left, top, right, bottom) = work;
    let width = size.0.min(i32::MAX as u32) as i32;
    let height = size.1.min(i32::MAX as u32) as i32;
    let x = left + ((right - left - width) / 2).max(0);
    let y = top + ((bottom - top - height) / 2).max(0);
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::center_in_work_area;

    #[test]
    fn centers_on_secondary_monitor_with_negative_coordinates() {
        assert_eq!(
            center_in_work_area((-1920, 0, 0, 1040), (800, 600)),
            (-1360, 220)
        );
    }

    #[test]
    fn oversized_window_starts_at_work_area_origin() {
        assert_eq!(
            center_in_work_area((1920, 100, 2520, 500), (800, 600)),
            (1920, 100)
        );
    }
}
