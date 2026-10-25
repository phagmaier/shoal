use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    str::FromStr,
};

use serde::{
    Deserialize, Deserializer,
    de::{DeserializeSeed, Error as _, MapAccess, Visitor},
};

use crate::Chord;

macro_rules! actions {
    ($($variant:ident => $name:literal),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub enum Action { $($variant),+ }

        impl Action {
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $name),+ }
            }
        }

        impl FromStr for Action {
            type Err = ActionError;
            fn from_str(input: &str) -> Result<Self, Self::Err> {
                match input { $($name => Ok(Self::$variant)),+, _ => Err(ActionError) }
            }
        }
    };
}

actions! {
    TabNew => "tab.new", TabClose => "tab.close", TabNext => "tab.next", TabPrev => "tab.prev",
    TabGoto1 => "tab.goto_1", TabGoto2 => "tab.goto_2", TabGoto3 => "tab.goto_3",
    TabGoto4 => "tab.goto_4", TabGoto5 => "tab.goto_5", TabGoto6 => "tab.goto_6",
    TabGoto7 => "tab.goto_7", TabGoto8 => "tab.goto_8", TabGoto9 => "tab.goto_9",
    TabMoveLeft => "tab.move_left", TabMoveRight => "tab.move_right", TabRename => "tab.rename",
    PaneSplitRight => "pane.split_right", PaneSplitDown => "pane.split_down", PaneClose => "pane.close",
    PaneFocusLeft => "pane.focus_left", PaneFocusRight => "pane.focus_right",
    PaneFocusUp => "pane.focus_up", PaneFocusDown => "pane.focus_down",
    PaneResizeLeft => "pane.resize_left", PaneResizeRight => "pane.resize_right",
    PaneResizeUp => "pane.resize_up", PaneResizeDown => "pane.resize_down", PaneZoom => "pane.zoom",
    SessionSwitcher => "session.switcher", SessionNext => "session.next", SessionPrev => "session.prev",
    WindowNew => "window.new", ClipboardCopy => "clipboard.copy", ClipboardPaste => "clipboard.paste",
    FontIncrease => "font.increase", FontDecrease => "font.decrease", FontReset => "font.reset",
    ScrollPageUp => "scroll.page_up", ScrollPageDown => "scroll.page_down",
    ScrollTop => "scroll.top", ScrollBottom => "scroll.bottom",
    ScrollPrevPrompt => "scroll.prev_prompt", ScrollNextPrompt => "scroll.next_prompt",
    ConfigEdit => "config.edit",
}

#[derive(Debug, thiserror::Error)]
#[error("unknown action id")]
pub struct ActionError;

/// Effective bindings after applying per-chord overrides to the defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keymap(BTreeMap<Chord, Action>);

impl Keymap {
    pub fn get(&self, chord: &Chord) -> Option<Action> {
        self.0.get(chord).copied()
    }
    pub fn iter(&self) -> impl Iterator<Item = (&Chord, &Action)> {
        self.0.iter()
    }
}

impl Default for Keymap {
    fn default() -> Self {
        Self(
            DEFAULTS
                .iter()
                .map(|(chord, action)| {
                    (
                        chord.parse().expect("invariant: default chord is valid"),
                        *action,
                    )
                })
                .collect(),
        )
    }
}

impl<'de> Deserialize<'de> for Keymap {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        struct Bindings;
        impl<'de> Visitor<'de> for Bindings {
            type Value = Keymap;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a table of key chords and actions")
            }
            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut keymap = Keymap::default();
                let mut seen = BTreeSet::new();
                while let Some(chord) = map.next_key::<Chord>()? {
                    // Report duplicate-chord errors while decoding their value,
                    // so the TOML deserializer attaches the offending entry's span.
                    let action = map.next_value_seed(BindingSeed {
                        duplicate: !seen.insert(chord),
                    })?;
                    match action {
                        Some(action) => {
                            keymap.0.insert(chord, action);
                        }
                        None => {
                            keymap.0.remove(&chord);
                        }
                    }
                }
                Ok(keymap)
            }
        }
        de.deserialize_map(Bindings)
    }
}

struct BindingSeed {
    duplicate: bool,
}
impl<'de> DeserializeSeed<'de> for BindingSeed {
    type Value = Option<Action>;
    fn deserialize<D: Deserializer<'de>>(self, de: D) -> Result<Self::Value, D::Error> {
        if self.duplicate {
            return Err(D::Error::custom("duplicate normalized key chord"));
        }
        let value = String::deserialize(de)?;
        if value == "none" {
            return Ok(None);
        }
        value.parse().map(Some).map_err(D::Error::custom)
    }
}

const DEFAULTS: &[(&str, Action)] = &[
    ("ctrl+shift+t", Action::TabNew),
    ("ctrl+alt+w", Action::TabClose),
    ("ctrl+tab", Action::TabNext),
    ("ctrl+shift+tab", Action::TabPrev),
    ("ctrl+pagedown", Action::TabNext),
    ("ctrl+pageup", Action::TabPrev),
    ("alt+1", Action::TabGoto1),
    ("alt+2", Action::TabGoto2),
    ("alt+3", Action::TabGoto3),
    ("alt+4", Action::TabGoto4),
    ("alt+5", Action::TabGoto5),
    ("alt+6", Action::TabGoto6),
    ("alt+7", Action::TabGoto7),
    ("alt+8", Action::TabGoto8),
    ("alt+9", Action::TabGoto9),
    ("ctrl+shift+pageup", Action::TabMoveLeft),
    ("ctrl+shift+pagedown", Action::TabMoveRight),
    ("ctrl+shift+r", Action::TabRename),
    ("ctrl+shift+o", Action::PaneSplitRight),
    ("ctrl+shift+e", Action::PaneSplitDown),
    ("ctrl+shift+w", Action::PaneClose),
    ("ctrl+alt+left", Action::PaneFocusLeft),
    ("ctrl+alt+right", Action::PaneFocusRight),
    ("ctrl+alt+up", Action::PaneFocusUp),
    ("ctrl+alt+down", Action::PaneFocusDown),
    ("ctrl+alt+shift+left", Action::PaneResizeLeft),
    ("ctrl+alt+shift+right", Action::PaneResizeRight),
    ("ctrl+alt+shift+up", Action::PaneResizeUp),
    ("ctrl+alt+shift+down", Action::PaneResizeDown),
    ("ctrl+shift+enter", Action::PaneZoom),
    ("ctrl+shift+s", Action::SessionSwitcher),
    ("ctrl+alt+pagedown", Action::SessionNext),
    ("ctrl+alt+pageup", Action::SessionPrev),
    ("ctrl+shift+n", Action::WindowNew),
    ("ctrl+shift+c", Action::ClipboardCopy),
    ("ctrl+insert", Action::ClipboardCopy),
    ("ctrl+shift+v", Action::ClipboardPaste),
    ("shift+insert", Action::ClipboardPaste),
    ("ctrl+=", Action::FontIncrease),
    ("ctrl+-", Action::FontDecrease),
    ("ctrl+0", Action::FontReset),
    ("shift+pageup", Action::ScrollPageUp),
    ("shift+pagedown", Action::ScrollPageDown),
    ("shift+home", Action::ScrollTop),
    ("shift+end", Action::ScrollBottom),
    ("ctrl+shift+up", Action::ScrollPrevPrompt),
    ("ctrl+shift+down", Action::ScrollNextPrompt),
    ("ctrl+shift+,", Action::ConfigEdit),
];
