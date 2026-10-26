//! Keystrokes as config chords, and chords as key caps (M1 spec §7.2, §10).

use gpui::Keystroke;
use shoal_config::{Action, Chord, Key, Keymap};

pub fn resolve(stroke: &Keystroke, held: gpui::Modifiers, keymap: &Keymap) -> Option<Action> {
    // Honor an explicitly bound symbol first, then try its Shift+key form.
    if let Some(action) = chord(stroke).and_then(|c| keymap.get(&c)) {
        return Some(action);
    }
    let mut stroke = stroke.clone();
    stroke.modifiers.shift |= held.shift;
    chord(&stroke).and_then(|c| keymap.get(&c))
}

/// The chord a keystroke spells, if the config could name it.
pub fn chord(keystroke: &Keystroke) -> Option<Chord> {
    let modifiers = &keystroke.modifiers;
    let mut spelled = String::new();
    for (held, name) in [
        (modifiers.control, "ctrl+"),
        (modifiers.shift, "shift+"),
        (modifiers.alt, "alt+"),
        (modifiers.platform, "super+"),
    ] {
        if held {
            spelled.push_str(name);
        }
    }
    let key = if modifiers.shift {
        unshift(&keystroke.key)
    } else {
        &keystroke.key
    };
    spelled.push_str(key);
    spelled.parse().ok()
}

/// GPUI's Linux backend names shifted punctuation by its symbol, but
/// config chords name the key along with Shift. Letters are already lower case.
pub fn unshift(key: &str) -> &str {
    match key {
        "~" => "`",
        "!" => "1",
        "@" => "2",
        "#" => "3",
        "$" => "4",
        "%" => "5",
        "^" => "6",
        "&" => "7",
        "*" => "8",
        "(" => "9",
        ")" => "0",
        "_" => "-",
        "+" => "=",
        "{" => "[",
        "}" => "]",
        "|" => "\\",
        ":" => ";",
        "\"" => "'",
        "<" => ",",
        ">" => ".",
        "?" => "/",
        _ => key,
    }
}

/// The first chord bound to `action`, for a button's tooltip.
pub fn shortcut(keymap: &Keymap, action: Action) -> Option<Chord> {
    keymap
        .iter()
        .find(|(_, bound)| **bound == action)
        .map(|(chord, _)| *chord)
}

/// A chord's key caps, modifiers first: `Ctrl`, `Shift`, `O`.
pub fn caps(chord: &Chord) -> Vec<String> {
    let modifiers = chord.modifiers;
    let mut caps: Vec<String> = [
        (modifiers.ctrl, "Ctrl"),
        (modifiers.shift, "Shift"),
        (modifiers.alt, "Alt"),
        (modifiers.super_key, "Super"),
    ]
    .into_iter()
    .filter(|(held, _)| *held)
    .map(|(_, name)| name.to_owned())
    .collect();
    caps.push(match chord.key {
        Key::Character(c) => c.to_uppercase().collect(),
        Key::Function(n) => format!("F{n}"),
        Key::Enter => "Enter".into(),
        Key::Tab => "Tab".into(),
        Key::Escape => "Esc".into(),
        Key::Space => "Space".into(),
        Key::Backspace => "Backspace".into(),
        Key::Insert => "Insert".into(),
        Key::Delete => "Delete".into(),
        Key::Home => "Home".into(),
        Key::End => "End".into(),
        Key::PageUp => "PgUp".into(),
        Key::PageDown => "PgDn".into(),
        Key::Up => "↑".into(),
        Key::Down => "↓".into(),
        Key::Left => "←".into(),
        Key::Right => "→".into(),
    });
    caps
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use gpui::Modifiers;
    use shoal_config::Config;

    use super::*;

    fn keystroke(key: &str, modifiers: Modifiers) -> Keystroke {
        Keystroke {
            modifiers,
            key: key.into(),
            key_char: None,
        }
    }

    fn ctrl_shift() -> Modifiers {
        Modifiers {
            control: true,
            shift: true,
            ..Modifiers::default()
        }
    }

    #[test]
    fn keystrokes_find_their_default_bindings() {
        let keymap = Keymap::default();
        let action = |key, modifiers| keymap.get(&chord(&keystroke(key, modifiers))?);
        // GPUI names a letter by its lowercase key, with Shift held.
        assert_eq!(action("r", ctrl_shift()), Some(Action::TabRename));
        assert_eq!(action("enter", ctrl_shift()), Some(Action::PaneZoom));
        assert_eq!(
            action("=", Modifiers::control()),
            Some(Action::FontIncrease)
        );
        let ctrl_alt_shift = Modifiers {
            alt: true,
            ..ctrl_shift()
        };
        assert_eq!(action("left", ctrl_alt_shift), Some(Action::PaneResizeLeft));
        assert_eq!(action("r", Modifiers::control()), None);
        // A key the config can't name is no chord.
        assert_eq!(chord(&keystroke("back", Modifiers::none())), None);
    }

    #[test]
    fn shifted_punctuation_resolves_the_config_chord() {
        let keymap = Keymap::default();
        // Actual Linux event: GPUI removed Shift, but the modifier-change
        // event retained it. Both must participate in dispatch.
        assert_eq!(
            resolve(&keystroke("<", Modifiers::control()), ctrl_shift(), &keymap),
            Some(Action::ConfigEdit)
        );
        assert_eq!(
            resolve(
                &keystroke("insert", Modifiers::control()),
                Modifiers::none(),
                &keymap
            ),
            Some(Action::ClipboardCopy)
        );
        assert_eq!(
            resolve(
                &keystroke("insert", Modifiers::shift()),
                Modifiers::none(),
                &keymap
            ),
            Some(Action::ClipboardPaste)
        );
        assert_eq!(
            chord(&keystroke("<", ctrl_shift())).and_then(|c| keymap.get(&c)),
            Some(Action::ConfigEdit)
        );
        let remapped = Config::parse(
            Path::new("config.toml"),
            "[keybinds]\n\"ctrl+shift+r\" = \"none\"\n\"ctrl+shift+;\" = \"tab.rename\"\n",
        )
        .unwrap()
        .config
        .keybinds;
        assert_eq!(
            chord(&keystroke("r", ctrl_shift())).and_then(|c| remapped.get(&c)),
            None
        );
        assert_eq!(
            chord(&keystroke(":", ctrl_shift())).and_then(|c| remapped.get(&c)),
            Some(Action::TabRename)
        );
        assert_eq!(
            chord(&keystroke("<", Modifiers::none())).unwrap().key,
            Key::Character('<')
        );
    }

    #[test]
    fn tooltips_show_the_bound_chord_as_caps() {
        let keymap = Keymap::default();
        let caps_of = |action| shortcut(&keymap, action).map(|chord| caps(&chord));
        assert_eq!(
            caps_of(Action::PaneSplitRight).unwrap(),
            ["Ctrl", "Shift", "O"]
        );
        assert_eq!(
            caps_of(Action::PaneSplitDown).unwrap(),
            ["Ctrl", "Shift", "E"]
        );
        assert_eq!(caps_of(Action::TabNew).unwrap(), ["Ctrl", "Shift", "T"]);
        let chord = |text: &str| text.parse::<Chord>().unwrap();
        assert_eq!(
            caps(&chord("ctrl+alt+shift+up")),
            ["Ctrl", "Shift", "Alt", "↑"]
        );
        assert_eq!(caps(&chord("super+f2")), ["Super", "F2"]);
        assert_eq!(caps(&chord("ctrl+pagedown")), ["Ctrl", "PgDn"]);
        // An action bound to nothing has no shortcut.
        let unbound = Config::parse(
            Path::new("config.toml"),
            "[keybinds]\n\"ctrl+shift+o\" = \"none\"\n",
        )
        .unwrap()
        .config
        .keybinds;
        assert_eq!(shortcut(&unbound, Action::PaneSplitRight), None);
    }
}
