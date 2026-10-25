//! Encoding keys, focus changes and pastes into the bytes a program expects,
//! using the terminal's current modes (M1 spec §5.6, §7.4). Every encoder
//! appends to `out` and appends nothing when the program doesn't want the
//! event.

use libghostty_vt::{
    focus,
    key::{self, Encoder, Event},
    paste,
    terminal::Mode as GhosttyMode,
};

use crate::{TermError, Terminal};

macro_rules! keys {
    ($($(#[$meta:meta])* $name:ident),* $(,)?) => {
        /// A physical key, named as in the W3C UI Events `code` values. It is
        /// the key's position, independent of the keyboard layout.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
        pub enum Key {
            $($(#[$meta])* $name),*
        }

        impl Key {
            fn ghostty(self) -> key::Key {
                match self {
                    $(Self::$name => key::Key::$name),*
                }
            }
        }
    };
}

keys! {
    #[default]
    Unidentified,
    // Writing system keys.
    Backquote, Backslash, BracketLeft, BracketRight, Comma, Equal, IntlBackslash,
    IntlRo, IntlYen, Minus, Period, Quote, Semicolon, Slash, Space,
    Digit0, Digit1, Digit2, Digit3, Digit4, Digit5, Digit6, Digit7, Digit8, Digit9,
    A, B, C, D, E, F, G, H, I, J, K, L, M, N, O, P, Q, R, S, T, U, V, W, X, Y, Z,
    // Modifier and lock keys.
    AltLeft, AltRight, ControlLeft, ControlRight, MetaLeft, MetaRight, ShiftLeft,
    ShiftRight, CapsLock, NumLock, ScrollLock, ContextMenu,
    // Control, editing and navigation keys.
    Backspace, Enter, Tab, Escape, Delete, Insert, Home, End, PageUp, PageDown,
    ArrowUp, ArrowDown, ArrowLeft, ArrowRight, PrintScreen, Pause,
    // The numeric keypad.
    Numpad0, Numpad1, Numpad2, Numpad3, Numpad4, Numpad5, Numpad6, Numpad7, Numpad8,
    Numpad9, NumpadAdd, NumpadSubtract, NumpadMultiply, NumpadDivide, NumpadDecimal,
    NumpadComma, NumpadEnter, NumpadEqual, NumpadSeparator, NumpadUp, NumpadDown,
    NumpadLeft, NumpadRight, NumpadBegin, NumpadHome, NumpadEnd, NumpadInsert,
    NumpadDelete, NumpadPageUp, NumpadPageDown,
    // Function keys.
    F1, F2, F3, F4, F5, F6, F7, F8, F9, F10, F11, F12, F13, F14, F15, F16, F17, F18,
    F19, F20, F21, F22, F23, F24, F25,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyAction {
    #[default]
    Press,
    Repeat,
    Release,
}

/// Modifiers held, and lock keys active, during an input event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[expect(clippy::struct_excessive_bools, reason = "independent modifier keys")]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub super_key: bool,
    pub caps_lock: bool,
    pub num_lock: bool,
}

impl Mods {
    pub(crate) fn ghostty(self) -> key::Mods {
        let mut mods = key::Mods::empty();
        for (held, flag) in [
            (self.shift, key::Mods::SHIFT),
            (self.ctrl, key::Mods::CTRL),
            (self.alt, key::Mods::ALT),
            (self.super_key, key::Mods::SUPER),
            (self.caps_lock, key::Mods::CAPS_LOCK),
            (self.num_lock, key::Mods::NUM_LOCK),
        ] {
            mods.set(flag, held);
        }
        mods
    }
}

/// One key event for [`Terminal::encode_key`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KeyEvent<'a> {
    pub key: Key,
    pub action: KeyAction,
    pub mods: Mods,
    /// The modifiers the keyboard layout used up producing `text`, such as
    /// Shift for `"!"` on Shift+1. The Kitty protocol sends such a key as
    /// plain text instead of an escape code.
    pub consumed: Mods,
    /// The text the key produces in the current layout before Ctrl or Alt
    /// change it, such as `"a"` for Ctrl+A or `"A"` for Shift+A. `None` for
    /// keys that produce no text and for control characters.
    pub text: Option<&'a str>,
    /// The character the key produces without Shift, such as `'a'` for
    /// Shift+A. The Kitty keyboard protocol reports keys by it.
    pub unshifted: Option<char>,
}

pub(crate) struct KeyEncoder {
    encoder: Encoder<'static>,
    event: Event<'static>,
}

impl std::fmt::Debug for KeyEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KeyEncoder")
    }
}

impl Terminal {
    /// Append the bytes for a key event under the terminal's current modes:
    /// cursor and keypad application modes, modifyOtherKeys and the Kitty
    /// keyboard flags the program pushed. Some events, such as a lone
    /// modifier press outside the Kitty protocol, produce nothing.
    pub fn encode_key(&mut self, event: &KeyEvent<'_>, out: &mut Vec<u8>) -> Result<(), TermError> {
        if self.view.keys.is_none() {
            self.view.keys = Some(KeyEncoder {
                encoder: Encoder::new()?,
                event: Event::new()?,
            });
        }
        let keys = self
            .view
            .keys
            .as_mut()
            .expect("invariant: the key encoder was just made");
        keys.event
            .set_key(event.key.ghostty())
            .set_action(match event.action {
                KeyAction::Press => key::Action::Press,
                KeyAction::Repeat => key::Action::Repeat,
                KeyAction::Release => key::Action::Release,
            })
            .set_mods(event.mods.ghostty())
            .set_consumed_mods(event.consumed.ghostty())
            .set_utf8(event.text)
            .set_unshifted_codepoint(event.unshifted.unwrap_or('\0'));
        keys.encoder.set_options_from_terminal(&self.inner);
        Ok(keys.encoder.encode_to_vec(&keys.event, out)?)
    }

    /// Append the focus report for a pane or window focus change, if the
    /// program enabled focus reporting (mode 1004).
    pub fn encode_focus(&self, focused: bool, out: &mut Vec<u8>) -> Result<(), TermError> {
        if !self.inner.mode(GhosttyMode::FOCUS_EVENT)? {
            return Ok(());
        }
        let event = if focused {
            focus::Event::Gained
        } else {
            focus::Event::Lost
        };
        let mut buf = [0; 8];
        let written = event.encode(&mut buf)?;
        out.extend_from_slice(&buf[..written]);
        Ok(())
    }

    /// Append pasted text. Control bytes that could end the paste early or
    /// inject commands, such as ESC, become spaces. If the program enabled
    /// bracketed paste (mode 2004) the text is wrapped in its markers;
    /// otherwise newlines become carriage returns.
    pub fn encode_paste(&self, text: &str, out: &mut Vec<u8>) -> Result<(), TermError> {
        let bracketed = self.inner.mode(GhosttyMode::BRACKETED_PASTE)?;
        let mut data = text.as_bytes().to_vec();
        // The data, plus the start and end markers.
        let mut needed = data.len() + 12;
        loop {
            let start = out.len();
            out.resize(start + needed, 0);
            match paste::encode(&mut data, bracketed, &mut out[start..]) {
                Ok(written) => {
                    out.truncate(start + written);
                    return Ok(());
                }
                Err(libghostty_vt::Error::OutOfSpace { required }) if required > needed => {
                    out.truncate(start);
                    needed = required;
                }
                Err(error) => {
                    out.truncate(start);
                    return Err(error.into());
                }
            }
        }
    }
}
