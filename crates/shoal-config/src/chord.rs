use std::{fmt, str::FromStr};

use serde::{Deserialize, Deserializer, de::Error as _};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub super_key: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Key {
    Character(char),
    Function(u8),
    Enter,
    Tab,
    Escape,
    Space,
    Backspace,
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Chord {
    pub modifiers: Modifiers,
    pub key: Key,
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ChordError(&'static str);

impl FromStr for Chord {
    type Err = ChordError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let input = input.to_lowercase();
        let (modifiers, key) = if input == "+" {
            ("", "+")
        } else if let Some(modifiers) = input.strip_suffix("++") {
            (modifiers, "+")
        } else {
            input.rsplit_once('+').unwrap_or(("", &input))
        };
        let mut parsed = Modifiers::default();
        if !modifiers.is_empty() {
            for modifier in modifiers.split('+') {
                let flag = match modifier {
                    "ctrl" => &mut parsed.ctrl,
                    "shift" => &mut parsed.shift,
                    "alt" => &mut parsed.alt,
                    "super" => &mut parsed.super_key,
                    _ => return Err(ChordError("unknown chord modifier")),
                };
                if std::mem::replace(flag, true) {
                    return Err(ChordError("repeated chord modifier"));
                }
            }
        }
        let key = match key {
            "enter" => Key::Enter,
            "tab" => Key::Tab,
            "escape" => Key::Escape,
            "space" => Key::Space,
            "backspace" => Key::Backspace,
            "insert" => Key::Insert,
            "delete" => Key::Delete,
            "home" => Key::Home,
            "end" => Key::End,
            "pageup" => Key::PageUp,
            "pagedown" => Key::PageDown,
            "up" => Key::Up,
            "down" => Key::Down,
            "left" => Key::Left,
            "right" => Key::Right,
            "f1" => Key::Function(1),
            "f2" => Key::Function(2),
            "f3" => Key::Function(3),
            "f4" => Key::Function(4),
            "f5" => Key::Function(5),
            "f6" => Key::Function(6),
            "f7" => Key::Function(7),
            "f8" => Key::Function(8),
            "f9" => Key::Function(9),
            "f10" => Key::Function(10),
            "f11" => Key::Function(11),
            "f12" => Key::Function(12),
            _ => {
                let mut chars = key.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) if c.is_alphanumeric() || c.is_ascii_punctuation() => {
                        Key::Character(c)
                    }
                    _ => return Err(ChordError("unknown chord key")),
                }
            }
        };
        // A leading separator isn't a modifier, and `++` requires a modifier.
        if modifiers.is_empty() && input != "+" && input.contains('+') {
            return Err(ChordError("empty chord modifier"));
        }
        Ok(Self {
            modifiers: parsed,
            key,
        })
    }
}

impl<'de> Deserialize<'de> for Chord {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        String::deserialize(de)?.parse().map_err(D::Error::custom)
    }
}

impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (active, name) in [
            (self.modifiers.ctrl, "ctrl"),
            (self.modifiers.shift, "shift"),
            (self.modifiers.alt, "alt"),
            (self.modifiers.super_key, "super"),
        ] {
            if active {
                write!(f, "{name}+")?;
            }
        }
        let name = match self.key {
            Key::Character(c) => return write!(f, "{c}"),
            Key::Function(n) => return write!(f, "f{n}"),
            Key::Enter => "enter",
            Key::Tab => "tab",
            Key::Escape => "escape",
            Key::Space => "space",
            Key::Backspace => "backspace",
            Key::Insert => "insert",
            Key::Delete => "delete",
            Key::Home => "home",
            Key::End => "end",
            Key::PageUp => "pageup",
            Key::PageDown => "pagedown",
            Key::Up => "up",
            Key::Down => "down",
            Key::Left => "left",
            Key::Right => "right",
        };
        f.write_str(name)
    }
}
