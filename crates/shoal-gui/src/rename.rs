//! The target and text of an inline tab rename (§7.2).
pub use crate::field::Outcome;
use crate::field::TextField;
use shoal_model::TabId;

pub struct Rename {
    pub tab: TabId,
    pub field: TextField,
}

impl Rename {
    pub fn new(tab: TabId, label: &str) -> Self {
        Self {
            tab,
            field: TextField::new(label),
        }
    }

    pub fn name(&self) -> Option<Option<String>> {
        self.field.name()
    }
}
