//! Read the foreground text selection through Windows UI Automation without
//! changing the clipboard or sending simulated keystrokes.

#[cfg(windows)]
pub(crate) fn cursor_position() -> Option<(i32, i32)> {
    use windows_sys::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetCursorPos};
    let mut point = POINT { x: 0, y: 0 };
    (unsafe { GetCursorPos(&mut point) } != 0).then_some((point.x, point.y))
}

#[cfg(not(windows))]
pub(crate) fn cursor_position() -> Option<(i32, i32)> {
    None
}

#[cfg(windows)]
pub(crate) fn read_selected_text() -> Option<String> {
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize,
    };
    use windows::Win32::UI::Accessibility::{CUIAutomation, IUIAutomation};
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    let result = (|| {
        let automation: IUIAutomation =
            unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER) }.ok()?;
        let walker = unsafe { automation.RawViewWalker() }.ok()?;
        let focused = unsafe { automation.GetFocusedElement() }.ok();
        if let Some(element) = focused
            && let Some(text) = selected_from_element(&walker, element)
        {
            return Some(text);
        }
        let (x, y) = cursor_position()?;
        let point = windows::Win32::Foundation::POINT { x, y };
        let element = unsafe { automation.ElementFromPoint(point) }.ok()?;
        selected_from_element(&walker, element)
    })();
    if initialized {
        unsafe { CoUninitialize() };
    }
    result
}

#[cfg(windows)]
fn selected_from_element(
    walker: &windows::Win32::UI::Accessibility::IUIAutomationTreeWalker,
    element: windows::Win32::UI::Accessibility::IUIAutomationElement,
) -> Option<String> {
    use windows::Win32::UI::Accessibility::{IUIAutomationTextPattern, UIA_TextPatternId};
    let mut current = Some(element);
    for _ in 0..8 {
        let element = current.take()?;
        // Password fields should never be sent to a translation provider.
        if unsafe { element.CurrentIsPassword() }.ok()?.as_bool() {
            return None;
        }
        if let Ok(pattern) =
            unsafe { element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) }
            && let Ok(ranges) = unsafe { pattern.GetSelection() }
            && let Ok(count) = unsafe { ranges.Length() }
            && (1..=20).contains(&count)
        {
            let mut pieces = Vec::with_capacity(count as usize);
            for index in 0..count {
                if let Ok(range) = unsafe { ranges.GetElement(index) }
                    && let Ok(text) = unsafe { range.GetText(8001) }
                {
                    let piece = text.to_string();
                    if !piece.trim().is_empty() {
                        pieces.push(piece);
                    }
                }
            }
            let selected = pieces.join("\n");
            let selected = selected.trim();
            if !selected.is_empty() && selected.chars().count() <= 4_000 {
                return Some(selected.to_owned());
            }
        }
        current = unsafe { walker.GetParentElement(&element) }.ok();
    }
    None
}

#[cfg(not(windows))]
pub(crate) fn read_selected_text() -> Option<String> {
    None
}
