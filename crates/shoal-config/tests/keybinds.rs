use proptest::prelude::*;
use shoal_config::{Chord, Key, Keymap, Modifiers};

#[test]
fn chords_match_layout_keys_and_normalize_case_and_modifier_order() {
    let expected = Chord {
        modifiers: Modifiers {
            ctrl: true,
            shift: true,
            ..Modifiers::default()
        },
        key: Key::Character('t'),
    };
    assert_eq!("ctrl+shift+t".parse::<Chord>().unwrap(), expected);
    assert_eq!("SHIFT+CTRL+T".parse::<Chord>().unwrap(), expected);
    for (text, key) in [
        ("ctrl+=", '='),
        ("alt+-", '-'),
        ("ctrl+\\", '\\'),
        ("ctrl++", '+'),
        ("+", '+'),
        ("!", '!'),
    ] {
        assert_eq!(text.parse::<Chord>().unwrap().key, Key::Character(key));
    }
    assert_eq!("super+f12".parse::<Chord>().unwrap().key, Key::Function(12));
    assert!("super+f12".parse::<Chord>().unwrap().modifiers.super_key);
    assert_eq!("ctrl+É".parse::<Chord>().unwrap().key, Key::Character('é'));
    for (text, key) in [
        ("enter", Key::Enter),
        ("tab", Key::Tab),
        ("escape", Key::Escape),
        ("space", Key::Space),
        ("backspace", Key::Backspace),
        ("insert", Key::Insert),
        ("delete", Key::Delete),
        ("home", Key::Home),
        ("end", Key::End),
        ("pageup", Key::PageUp),
        ("pagedown", Key::PageDown),
        ("up", Key::Up),
        ("down", Key::Down),
        ("left", Key::Left),
        ("right", Key::Right),
    ] {
        assert_eq!(text.parse::<Chord>().unwrap().key, key);
    }
    for number in 1..=12 {
        assert_eq!(
            format!("f{number}").parse::<Chord>().unwrap().key,
            Key::Function(number)
        );
    }
}

#[test]
fn every_default_chord_maps_to_the_spec_action_and_no_default_uses_super() {
    let expected = include_str!("fixtures/default-keybinds.tsv");
    let keymap = Keymap::default();
    assert_eq!(keymap.iter().count(), 48);
    assert_eq!(expected.lines().count(), 48);
    for line in expected.lines() {
        let (chord, action) = line.split_once(' ').unwrap();
        let chord = chord.parse::<Chord>().unwrap();
        assert!(!chord.modifiers.super_key);
        assert_eq!(keymap.get(&chord).unwrap().as_str(), action, "{chord}");
    }
}

proptest! {
    #[test]
    fn arbitrary_text_never_panics_and_accepted_chords_have_canonical_round_trips(input in ".{0,100}") {
        if let Ok(chord) = input.parse::<Chord>() {
            prop_assert_eq!(chord.to_string().parse::<Chord>().unwrap(), chord);
            if let Key::Character(c) = chord.key {
                prop_assert!(c.is_alphanumeric() || c.is_ascii_punctuation());
                prop_assert!(!c.is_ascii_uppercase());
            }
        }
    }
}

#[test]
fn malformed_chords_are_errors() {
    for text in [
        "",
        "ctrl",
        "ctrl+",
        "ctrl+ctrl+t",
        "ctrl+CTRL+t",
        "meta+t",
        "ctrl+shift",
        "ctrl++t",
        "ctrl+f0",
        "f13",
        "f01",
        "ctrl+ab",
        "ctrl+ t",
        " enter",
        "ctrl+\n",
        "+t",
        "++",
    ] {
        assert!(text.parse::<Chord>().is_err(), "{text:?}");
    }
}
