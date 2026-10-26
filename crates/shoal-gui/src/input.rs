//! GPUI keys translated into the terminal encoder's public input types.
//! GPUI exposes logical key names, not hardware codes. Keep actual text
//! from the keyboard layout; infer encoder key identities from those names.

use gpui::{Keystroke, Modifiers};
use shoal_term::{Key, KeyAction, KeyEvent, Mods};

use crate::keys;

pub fn mods(value: Modifiers) -> Mods {
    Mods {
        shift: value.shift,
        ctrl: value.control,
        alt: value.alt,
        super_key: value.platform,
        ..Mods::default()
    }
}

pub struct InputKey {
    key: Key,
    text: Option<String>,
    unshifted: Option<char>,
    mods: Mods,
    consumed: Mods,
    action: KeyAction,
}

impl InputKey {
    pub fn new(stroke: &Keystroke, held: Modifiers, caps_lock: bool, action: KeyAction) -> Self {
        let mut modifiers = stroke.modifiers;
        // Linux GPUI removes Shift for punctuation (platform.rs). Its
        // modifier-change event still carries the actual held state.
        modifiers.shift |= held.shift;
        let mut mods = mods(modifiers);
        mods.caps_lock = caps_lock;
        let name = if modifiers.shift {
            keys::unshift(&stroke.key)
        } else {
            &stroke.key
        };
        let unshifted = if name == "space" {
            Some(' ')
        } else {
            let mut chars = name.chars();
            chars.next().filter(|_| chars.next().is_none())
        };
        let text = stroke.key_char.clone().or_else(|| {
            unshifted.map(|c| {
                if modifiers.shift && c.is_ascii_alphabetic() {
                    c.to_ascii_uppercase().to_string()
                } else if name == "space" {
                    " ".into()
                } else {
                    stroke.key.clone()
                }
            })
        });
        let consumed = Mods {
            shift: modifiers.shift && text.is_some(),
            ..Mods::default()
        };
        Self {
            key: key(name),
            text,
            unshifted,
            mods,
            consumed,
            action,
        }
    }

    pub fn event(&self) -> KeyEvent<'_> {
        KeyEvent {
            key: self.key,
            text: self.text.as_deref(),
            unshifted: self.unshifted,
            mods: self.mods,
            consumed: self.consumed,
            action: self.action,
        }
    }
}

fn key(name: &str) -> Key {
    match name {
        "enter" | "return" => Key::Enter,
        "tab" => Key::Tab,
        "escape" => Key::Escape,
        "space" => Key::Space,
        "backspace" => Key::Backspace,
        "delete" => Key::Delete,
        "insert" => Key::Insert,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "up" => Key::ArrowUp,
        "down" => Key::ArrowDown,
        "left" => Key::ArrowLeft,
        "right" => Key::ArrowRight,
        "print" => Key::PrintScreen,
        "pause" => Key::Pause,
        "a" => Key::A,
        "b" => Key::B,
        "c" => Key::C,
        "d" => Key::D,
        "e" => Key::E,
        "f" => Key::F,
        "g" => Key::G,
        "h" => Key::H,
        "i" => Key::I,
        "j" => Key::J,
        "k" => Key::K,
        "l" => Key::L,
        "m" => Key::M,
        "n" => Key::N,
        "o" => Key::O,
        "p" => Key::P,
        "q" => Key::Q,
        "r" => Key::R,
        "s" => Key::S,
        "t" => Key::T,
        "u" => Key::U,
        "v" => Key::V,
        "w" => Key::W,
        "x" => Key::X,
        "y" => Key::Y,
        "z" => Key::Z,
        "0" => Key::Digit0,
        "1" => Key::Digit1,
        "2" => Key::Digit2,
        "3" => Key::Digit3,
        "4" => Key::Digit4,
        "5" => Key::Digit5,
        "6" => Key::Digit6,
        "7" => Key::Digit7,
        "8" => Key::Digit8,
        "9" => Key::Digit9,
        "`" => Key::Backquote,
        "-" => Key::Minus,
        "=" => Key::Equal,
        "[" => Key::BracketLeft,
        "]" => Key::BracketRight,
        "\\" => Key::Backslash,
        ";" => Key::Semicolon,
        "'" => Key::Quote,
        "," => Key::Comma,
        "." => Key::Period,
        "/" => Key::Slash,
        "f1" => Key::F1,
        "f2" => Key::F2,
        "f3" => Key::F3,
        "f4" => Key::F4,
        "f5" => Key::F5,
        "f6" => Key::F6,
        "f7" => Key::F7,
        "f8" => Key::F8,
        "f9" => Key::F9,
        "f10" => Key::F10,
        "f11" => Key::F11,
        "f12" => Key::F12,
        "f13" => Key::F13,
        "f14" => Key::F14,
        "f15" => Key::F15,
        "f16" => Key::F16,
        "f17" => Key::F17,
        "f18" => Key::F18,
        "f19" => Key::F19,
        "f20" => Key::F20,
        "f21" => Key::F21,
        "f22" => Key::F22,
        "f23" => Key::F23,
        "f24" => Key::F24,
        "f25" => Key::F25,
        "add" => Key::NumpadAdd,
        "subtract" => Key::NumpadSubtract,
        "multiply" => Key::NumpadMultiply,
        "divide" => Key::NumpadDivide,
        "decimal" => Key::NumpadDecimal,
        "equal" => Key::NumpadEqual,
        _ => Key::Unidentified,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shoal_term::Terminal;

    fn encode(
        setup: &[u8],
        name: &str,
        text: Option<&str>,
        modifiers: Modifiers,
        held: Modifiers,
        action: KeyAction,
    ) -> Vec<u8> {
        let mut terminal = Terminal::new(80, 24, 10).unwrap();
        terminal.feed(setup);
        let stroke = Keystroke {
            key: name.into(),
            key_char: text.map(str::to_owned),
            modifiers,
        };
        let input = InputKey::new(&stroke, held, false, action);
        let mut out = Vec::new();
        terminal.encode_key(&input.event(), &mut out).unwrap();
        out
    }

    #[test]
    fn gpui_keys_keep_text_modifiers_and_terminal_modes() {
        let none = Modifiers::none();
        let press = KeyAction::Press;
        assert_eq!(
            encode(b"", "a", None, Modifiers::control(), none, press),
            b"\x01"
        );
        assert_eq!(
            encode(b"", "x", Some("x"), Modifiers::alt(), none, press),
            b"\x1bx"
        );
        assert_eq!(
            encode(b"", "e", Some("é"), none, none, press),
            "é".as_bytes()
        );
        assert_eq!(
            encode(b"\x1b[?1h", "up", None, none, none, press),
            b"\x1bOA"
        );
        assert_eq!(
            encode(b"\x1b[>3u", "escape", None, none, none, KeyAction::Release),
            b"\x1b[27;1:3u"
        );
        assert_eq!(
            encode(b"\x1b[>8u", "a", Some("A"), Modifiers::shift(), none, press),
            b"\x1b[97;2u"
        );
        assert_eq!(
            encode(b"\x1b[>1u", "!", Some("!"), none, Modifiers::shift(), press),
            b"!"
        );
    }
}
