//! 键盘和鼠标快捷键的统一格式与匹配规则。
//!
//! 键盘沿用 global-hotkey 的格式；鼠标使用 MouseLeft/MouseRight/MouseMiddle/
//! MouseX1/MouseX2、Mouse6～Mouse8，前面可添加 ctrl、alt、shift、super 修饰键。

use global_hotkey::hotkey::HotKey;
use std::fmt;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum MouseButton {
    Left,
    Right,
    Middle,
    X1,
    X2,
    Extra6,
    Extra7,
    Extra8,
}

impl MouseButton {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Left => "MouseLeft",
            Self::Right => "MouseRight",
            Self::Middle => "MouseMiddle",
            Self::X1 => "MouseX1",
            Self::X2 => "MouseX2",
            Self::Extra6 => "Mouse6",
            Self::Extra7 => "Mouse7",
            Self::Extra8 => "Mouse8",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "mouseleft" => Some(Self::Left),
            "mouseright" => Some(Self::Right),
            "mousemiddle" => Some(Self::Middle),
            "mousex1" | "mouse4" | "xbutton1" => Some(Self::X1),
            "mousex2" | "mouse5" | "xbutton2" => Some(Self::X2),
            "mouse6" => Some(Self::Extra6),
            "mouse7" => Some(Self::Extra7),
            "mouse8" => Some(Self::Extra8),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct MouseModifiers(u8);

impl MouseModifiers {
    const CTRL: u8 = 1;
    const ALT: u8 = 2;
    const SHIFT: u8 = 4;
    const SUPER: u8 = 8;

    pub(crate) fn new(ctrl: bool, alt: bool, shift: bool, super_key: bool) -> Self {
        Self(
            (if ctrl { Self::CTRL } else { 0 })
                | (if alt { Self::ALT } else { 0 })
                | (if shift { Self::SHIFT } else { 0 })
                | (if super_key { Self::SUPER } else { 0 }),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct MouseShortcut {
    pub(crate) button: MouseButton,
    pub(crate) modifiers: MouseModifiers,
}

impl MouseShortcut {
    pub(crate) fn matches(self, button: MouseButton, modifiers: MouseModifiers) -> bool {
        self.button == button && self.modifiers == modifiers
    }
}

impl fmt::Display for MouseShortcut {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (mask, name) in [
            (MouseModifiers::CTRL, "ctrl"),
            (MouseModifiers::ALT, "alt"),
            (MouseModifiers::SHIFT, "shift"),
            (MouseModifiers::SUPER, "super"),
        ] {
            if self.modifiers.0 & mask != 0 {
                write!(formatter, "{name}+")?;
            }
        }
        formatter.write_str(self.button.name())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shortcut {
    Keyboard(HotKey),
    Mouse(MouseShortcut),
}

impl Shortcut {
    pub(crate) fn keyboard(self) -> Option<HotKey> {
        match self {
            Self::Keyboard(key) => Some(key),
            Self::Mouse(_) => None,
        }
    }

    pub(crate) fn keyboard_id(self) -> Option<u32> {
        self.keyboard().map(|key| key.id())
    }

    pub(crate) fn mouse(self) -> Option<MouseShortcut> {
        match self {
            Self::Keyboard(_) => None,
            Self::Mouse(mouse) => Some(mouse),
        }
    }
}

impl FromStr for Shortcut {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let mut tokens = text.split('+').map(str::trim).collect::<Vec<_>>();
        let Some(last) = tokens.pop() else {
            return Err("快捷键不能为空".to_string());
        };
        if let Some(button) = MouseButton::parse(last) {
            let mut bits = 0_u8;
            for token in tokens {
                let bit = match token.to_ascii_lowercase().as_str() {
                    "ctrl" | "control" => MouseModifiers::CTRL,
                    "alt" => MouseModifiers::ALT,
                    "shift" => MouseModifiers::SHIFT,
                    "super" | "win" => MouseModifiers::SUPER,
                    _ => return Err(format!("不支持的鼠标快捷键修饰键：{token}")),
                };
                if bits & bit != 0 {
                    return Err(format!("鼠标快捷键修饰键重复：{token}"));
                }
                bits |= bit;
            }
            Ok(Self::Mouse(MouseShortcut {
                button,
                modifiers: MouseModifiers(bits),
            }))
        } else {
            text.parse::<HotKey>()
                .map(Self::Keyboard)
                .map_err(|error| error.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MouseButton, MouseModifiers, Shortcut};

    #[test]
    fn mouse_buttons_and_modifiers_round_trip() {
        for button in [
            MouseButton::Left,
            MouseButton::Right,
            MouseButton::Middle,
            MouseButton::X1,
            MouseButton::X2,
            MouseButton::Extra6,
            MouseButton::Extra7,
            MouseButton::Extra8,
        ] {
            let text = format!("ctrl+alt+{}", button.name());
            let parsed: Shortcut = text.parse().unwrap();
            assert_eq!(parsed.mouse().unwrap().to_string(), text);
            assert!(
                parsed
                    .mouse()
                    .unwrap()
                    .matches(button, MouseModifiers::new(true, true, false, false))
            );
        }
        assert!("MouseX1".parse::<Shortcut>().unwrap().mouse().is_some());
        assert_eq!(
            "mouse4"
                .parse::<Shortcut>()
                .unwrap()
                .mouse()
                .unwrap()
                .button,
            MouseButton::X1
        );
        assert_eq!(
            "mouse5"
                .parse::<Shortcut>()
                .unwrap()
                .mouse()
                .unwrap()
                .button,
            MouseButton::X2
        );
    }

    #[test]
    fn invalid_mouse_modifier_is_rejected() {
        assert!("ctrl+ctrl+MouseX1".parse::<Shortcut>().is_err());
        assert!("unknown+MouseX2".parse::<Shortcut>().is_err());
        assert!(
            "ctrl+alt+enter"
                .parse::<Shortcut>()
                .unwrap()
                .keyboard()
                .is_some()
        );
        assert!("F13".parse::<Shortcut>().unwrap().keyboard().is_some());
        assert!("mouse9".parse::<Shortcut>().is_err());
    }
}
