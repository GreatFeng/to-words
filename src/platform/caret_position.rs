//! Windows 文字光标定位：依次尝试系统插入符、UI Automation、MSAA 和 IME。
//! 坐标始终使用物理像素，供 Tauri 窗口直接定位。

use windows_sys::Win32::{
    Foundation::{HWND, POINT},
    Graphics::Gdi::{
        ClientToScreen, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTONULL,
        MONITORINFO, MonitorFromPoint, MonitorFromWindow,
    },
    UI::WindowsAndMessaging::{GUITHREADINFO, GetCursorPos, GetForegroundWindow, GetGUIThreadInfo},
};

#[derive(Clone, Copy)]
struct CaretRect {
    left: i32,
    top: i32,
    bottom: i32,
}

pub(crate) fn query_position(width: f32, height: f32, scale: f32) -> Option<(i32, i32)> {
    let caret = unsafe {
        system_caret()
            .or_else(|| uia_caret())
            .or_else(|| msaa_caret())
            .or_else(|| ime_caret())
    };
    unsafe {
        let foreground = GetForegroundWindow();
        let mut monitor = if let Some(rect) = caret {
            MonitorFromPoint(
                POINT {
                    x: rect.left,
                    y: rect.bottom,
                },
                MONITOR_DEFAULTTONEAREST,
            )
        } else if foreground.is_null() {
            std::ptr::null_mut()
        } else {
            MonitorFromWindow(foreground, MONITOR_DEFAULTTONULL)
        };
        if monitor.is_null() {
            let mut cursor = POINT { x: 0, y: 0 };
            if GetCursorPos(&mut cursor) == 0 {
                return None;
            }
            monitor = MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST);
        }
        if monitor.is_null() {
            return None;
        }
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return None;
        }
        let work = info.rcWork;
        let width = (width * scale.max(1.0)).round() as i32;
        let height = (height * scale.max(1.0)).round() as i32;
        let x = caret
            .map_or(work.left + 12, |rect| rect.left)
            .clamp(work.left, (work.right - width).max(work.left));
        let desired_y = caret.map_or(work.bottom - 52 - height, |rect| {
            if rect.bottom + 8 + height <= work.bottom {
                rect.bottom + 8
            } else {
                rect.top - 8 - height
            }
        });
        Some((
            x,
            desired_y.clamp(work.top, (work.bottom - height).max(work.top)),
        ))
    }
}

unsafe fn system_caret() -> Option<CaretRect> {
    let mut info: GUITHREADINFO = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
    if unsafe { GetGUIThreadInfo(0, &mut info) } == 0 || info.hwndCaret.is_null() {
        return None;
    }
    let mut top = POINT {
        x: info.rcCaret.left,
        y: info.rcCaret.top,
    };
    let mut bottom = POINT {
        x: info.rcCaret.right,
        y: info.rcCaret.bottom,
    };
    if unsafe { ClientToScreen(info.hwndCaret, &mut top) } == 0
        || unsafe { ClientToScreen(info.hwndCaret, &mut bottom) } == 0
    {
        return None;
    }
    Some(CaretRect {
        left: top.x,
        top: top.y,
        bottom: bottom.y,
    })
}

unsafe fn uia_caret() -> Option<CaretRect> {
    use windows::Win32::{
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize,
        },
        UI::Accessibility::{CUIAutomation, IUIAutomation},
    };
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    let result = (|| {
        let automation: IUIAutomation =
            unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.ok()?;
        let focused = unsafe { automation.GetFocusedElement() }.ok()?;
        let walker = unsafe { automation.RawViewWalker() }.ok()?;
        let mut current = Some(focused);
        for _ in 0..10 {
            let element = current.take()?;
            if let Some(rect) = unsafe { uia_element_caret(&element) } {
                return Some(rect);
            }
            current = unsafe { walker.GetParentElement(&element) }.ok();
        }
        None
    })();
    if initialized {
        unsafe { CoUninitialize() };
    }
    result
}

unsafe fn uia_element_caret(
    element: &windows::Win32::UI::Accessibility::IUIAutomationElement,
) -> Option<CaretRect> {
    use windows::Win32::{
        Foundation::BOOL,
        UI::Accessibility::{
            IUIAutomationTextPattern, IUIAutomationTextPattern2, UIA_TextPattern2Id,
            UIA_TextPatternId,
        },
    };
    let caret = unsafe {
        element
            .GetCurrentPatternAs::<IUIAutomationTextPattern2>(UIA_TextPattern2Id)
            .ok()
            .and_then(|pattern| {
                let mut active = BOOL(0);
                pattern.GetCaretRange(&mut active).ok()
            })
    };
    if let Some(rect) = caret
        .as_ref()
        .and_then(|range| unsafe { uia_range_caret(range) })
    {
        return Some(rect);
    }
    let selected = unsafe {
        let pattern = element
            .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
            .ok()?;
        let ranges = pattern.GetSelection().ok()?;
        (ranges.Length().ok()? > 0)
            .then(|| ranges.GetElement(0).ok())
            .flatten()
    }?;
    unsafe { uia_range_caret(&selected) }
}

unsafe fn uia_range_caret(
    range: &windows::Win32::UI::Accessibility::IUIAutomationTextRange,
) -> Option<CaretRect> {
    use windows::Win32::System::Ole::SafeArrayDestroy;
    let rectangles = unsafe { range.GetBoundingRectangles() }.ok()?;
    if rectangles.is_null() {
        return None;
    }
    let result = unsafe { last_uia_rectangle(rectangles) };
    let _ = unsafe { SafeArrayDestroy(rectangles) };
    result
}

unsafe fn last_uia_rectangle(
    rectangles: *mut windows::Win32::System::Com::SAFEARRAY,
) -> Option<CaretRect> {
    use windows::Win32::System::Ole::{
        SafeArrayGetDim, SafeArrayGetElement, SafeArrayGetLBound, SafeArrayGetUBound,
    };
    if unsafe { SafeArrayGetDim(rectangles) } != 1 {
        return None;
    }
    let lower = unsafe { SafeArrayGetLBound(rectangles, 1) }.ok()?;
    let upper = unsafe { SafeArrayGetUBound(rectangles, 1) }.ok()?;
    let count = upper.checked_sub(lower)?.checked_add(1)?;
    if count < 4 || count % 4 != 0 {
        return None;
    }
    let mut values = [0.0_f64; 4];
    for (offset, value) in values.iter_mut().enumerate() {
        let index = upper - 3 + offset as i32;
        unsafe { SafeArrayGetElement(rectangles, &index, std::ptr::from_mut(value).cast()) }
            .ok()?;
    }
    let [left, top, _, height] = values;
    if !values.iter().all(|value| value.is_finite()) || height <= 0.0 {
        return None;
    }
    Some(CaretRect {
        left: left.round() as i32,
        top: top.round() as i32,
        bottom: (top + height).round() as i32,
    })
}

unsafe fn msaa_caret() -> Option<CaretRect> {
    use windows::{
        Win32::{
            Foundation::HWND as ComHwnd,
            System::{
                Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
                Variant::VARIANT,
            },
            UI::Accessibility::{AccessibleObjectFromWindow, IAccessible},
        },
        core::Interface,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::OBJID_CARET;
    let focused = unsafe { focused_window() }?;
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    let result = (|| {
        let mut raw = std::ptr::null_mut();
        unsafe {
            AccessibleObjectFromWindow(
                ComHwnd(focused),
                OBJID_CARET as u32,
                &IAccessible::IID,
                &mut raw,
            )
        }
        .ok()?;
        if raw.is_null() {
            return None;
        }
        let accessible = unsafe { IAccessible::from_raw(raw) };
        let child = VARIANT::from(0_i32);
        let (mut left, mut top, mut width, mut height) = (0, 0, 0, 0);
        unsafe { accessible.accLocation(&mut left, &mut top, &mut width, &mut height, &child) }
            .ok()?;
        (height > 0).then_some(CaretRect {
            left,
            top,
            bottom: top + height,
        })
    })();
    if initialized {
        unsafe { CoUninitialize() };
    }
    result
}

unsafe fn ime_caret() -> Option<CaretRect> {
    use windows_sys::Win32::UI::Input::Ime::{
        CANDIDATEFORM, CFS_CANDIDATEPOS, CFS_EXCLUDE, CFS_FORCE_POSITION, CFS_POINT, CFS_RECT,
        COMPOSITIONFORM, ImmGetCandidateWindow, ImmGetCompositionWindow, ImmGetContext,
        ImmReleaseContext,
    };
    let focused = unsafe { focused_window() }?;
    let input = unsafe { ImmGetContext(focused) };
    if input.is_null() {
        return None;
    }
    let mut point = None;
    let mut composition: COMPOSITIONFORM = unsafe { std::mem::zeroed() };
    if unsafe { ImmGetCompositionWindow(input, &mut composition) } != 0
        && matches!(
            composition.dwStyle,
            CFS_POINT | CFS_FORCE_POSITION | CFS_RECT
        )
    {
        point = Some(composition.ptCurrentPos);
    }
    if point.is_none() {
        let mut candidate: CANDIDATEFORM = unsafe { std::mem::zeroed() };
        if unsafe { ImmGetCandidateWindow(input, 0, &mut candidate) } != 0
            && matches!(candidate.dwStyle, CFS_CANDIDATEPOS | CFS_EXCLUDE)
        {
            point = Some(candidate.ptCurrentPos);
        }
    }
    unsafe { ImmReleaseContext(focused, input) };
    let mut point = point?;
    if unsafe { ClientToScreen(focused, &mut point) } == 0 {
        return None;
    }
    Some(CaretRect {
        left: point.x,
        top: point.y,
        bottom: point.y + 20,
    })
}

unsafe fn focused_window() -> Option<HWND> {
    let mut info: GUITHREADINFO = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
    if unsafe { GetGUIThreadInfo(0, &mut info) } == 0 || info.hwndFocus.is_null() {
        None
    } else {
        Some(info.hwndFocus)
    }
}
