//! Renaming a tab inline (M1 spec §7.2, §10 `tab.rename`): the text being
//! edited and what a key does to it. It starts with the whole label
//! selected, so typing replaces it. Committing an empty name clears the
//! tab's name, giving it back its automatic title.

use shoal_model::TabId;

#[derive(Debug, Clone, PartialEq)]
pub struct Rename {
    pub tab: TabId,
    text: String,
    /// A byte offset on a character boundary.
    caret: usize,
    /// The whole text is selected.
    all: bool,
    /// The label when editing began.
    original: String,
}

/// What a key did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Editing,
    /// Enter: the `tab.rename` name to send, `None` for no change.
    Commit(Option<Option<String>>),
    /// Escape.
    Cancel,
}

impl Rename {
    pub fn new(tab: TabId, label: &str) -> Self {
        Self {
            tab,
            text: label.to_owned(),
            caret: label.len(),
            all: true,
            original: label.to_owned(),
        }
    }

    /// The text before the caret, and after it; with the whole text
    /// selected, it is all before.
    pub fn parts(&self) -> (&str, &str) {
        self.text.split_at(self.caret)
    }

    pub fn all_selected(&self) -> bool {
        self.all && !self.text.is_empty()
    }

    /// Apply a key: GPUI's key name and the text it types, which is `None`
    /// for chords with Ctrl, Alt or Super.
    pub fn key(&mut self, key: &str, typed: Option<&str>) -> Outcome {
        match key {
            "enter" => return Outcome::Commit(self.name()),
            "escape" => return Outcome::Cancel,
            "backspace" | "delete" if self.all => self.clear(),
            "backspace" => {
                if let Some(before) = self.before() {
                    self.text.replace_range(before..self.caret, "");
                    self.caret = before;
                }
            }
            "delete" => {
                if let Some(after) = self.after() {
                    self.text.replace_range(self.caret..after, "");
                }
            }
            "left" if self.all => self.caret = 0,
            "left" => self.caret = self.before().unwrap_or(self.caret),
            "right" if self.all => self.caret = self.text.len(),
            "right" => self.caret = self.after().unwrap_or(self.caret),
            "home" => self.caret = 0,
            "end" => self.caret = self.text.len(),
            _ => match typed.filter(|text| !text.chars().any(char::is_control)) {
                Some(text) => {
                    if self.all {
                        self.clear();
                    }
                    self.text.insert_str(self.caret, text);
                    self.caret += text.len();
                }
                None => return Outcome::Editing,
            },
        }
        self.all = false;
        Outcome::Editing
    }

    /// The `tab.rename` name for the text now, `None` if it is unchanged.
    pub fn name(&self) -> Option<Option<String>> {
        let name = self.text.trim();
        if name == self.original.trim() {
            None
        } else if name.is_empty() {
            Some(None)
        } else {
            Some(Some(name.to_owned()))
        }
    }

    fn clear(&mut self) {
        self.text.clear();
        self.caret = 0;
    }

    fn before(&self) -> Option<usize> {
        self.text[..self.caret]
            .char_indices()
            .last()
            .map(|(at, _)| at)
    }

    fn after(&self) -> Option<usize> {
        let c = self.text[self.caret..].chars().next()?;
        Some(self.caret + c.len_utf8())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rename(label: &str) -> Rename {
        Rename::new(TabId::new(2), label)
    }

    /// Type each character of `text`.
    fn type_text(field: &mut Rename, text: &str) {
        for c in text.chars() {
            let typed = c.to_string();
            assert_eq!(field.key(&typed, Some(&typed)), Outcome::Editing);
        }
    }

    #[test]
    fn typing_replaces_the_selected_label() {
        let mut field = rename("zsh");
        assert!(field.all_selected());
        type_text(&mut field, "logs");
        assert_eq!(field.parts(), ("logs", ""));
        assert!(!field.all_selected());
        assert_eq!(
            field.key("enter", None),
            Outcome::Commit(Some(Some("logs".into())))
        );
    }

    #[test]
    fn the_caret_moves_and_deletes_by_character() {
        let mut field = rename("café");
        // Left first drops the selection and puts the caret at the start.
        field.key("left", None);
        assert_eq!(field.parts(), ("", "café"));
        field.key("end", None);
        field.key("left", None);
        assert_eq!(field.parts(), ("caf", "é"));
        field.key("delete", None);
        assert_eq!(field.parts(), ("caf", ""));
        field.key("backspace", None);
        field.key("home", None);
        field.key("right", None);
        type_text(&mut field, "ü");
        assert_eq!(field.parts(), ("cü", "a"));
        // At the ends, nothing more is deleted or passed.
        field.key("end", None);
        field.key("delete", None);
        field.key("right", None);
        assert_eq!(field.parts(), ("cüa", ""));
        // Chords type nothing, and neither do control characters.
        assert_eq!(field.key("a", None), Outcome::Editing);
        field.key("tab", Some("\t"));
        assert_eq!(field.parts(), ("cüa", ""));
    }

    #[test]
    fn an_empty_name_clears_and_an_unchanged_one_sends_nothing() {
        let mut field = rename("work");
        field.key("backspace", None);
        assert_eq!(field.parts(), ("", ""));
        assert_eq!(field.key("enter", None), Outcome::Commit(Some(None)));
        // Spaces around a name don't count, and the same name is no change.
        let mut same = rename("work");
        type_text(&mut same, " work ");
        assert_eq!(same.key("enter", None), Outcome::Commit(None));
        assert_eq!(rename("work").key("escape", None), Outcome::Cancel);
    }
}
