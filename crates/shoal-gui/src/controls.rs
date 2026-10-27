//! Small chrome controls shared by the bars, cards and tooltips (M1 spec
//! §7.2, §7.6, §7.7), so a shortcut or a button looks the same wherever the
//! window shows one.

use gpui::{Div, ElementId, SharedString, Stateful, div, prelude::*, px};
use shoal_config::Palette;

use crate::{chrome::Color, fonts, view::rgba};

/// Codicons `warning`, `arrow-down`, `search` and `add`, from the bundled
/// symbols font.
pub const WARNING_GLYPH: &str = "\u{ea6c}";
pub const DOWN_GLYPH: &str = "\u{ea9a}";
pub const SEARCH_GLYPH: &str = "\u{ea6d}";
pub const ADD_GLYPH: &str = "\u{ea60}";

/// The colors controls are drawn in, from the palette.
#[derive(Debug, Clone, Copy)]
pub struct Looks {
    pub cap_fill: Color,
    pub cap_text: Color,
    /// Hint labels, beside their caps.
    pub hint: Color,
    pub button: Color,
    pub button_hover: Color,
    /// Around caps and buttons, so they keep their shape where
    /// `lighter_background` is close to the surface, as in light themes.
    pub outline: Color,
    pub text: Color,
    pub accent: Color,
}

impl Looks {
    pub fn new(palette: &Palette) -> Self {
        Self {
            cap_fill: Color::opaque(palette.unfocused_border),
            cap_text: Color::opaque(palette.foreground),
            hint: Color::opaque(palette.inactive_text),
            button: Color::opaque(palette.unfocused_border),
            button_hover: Color {
                rgb: palette.accent,
                alpha: 0.28,
            },
            outline: Color {
                rgb: palette.inactive_text,
                alpha: 0.35,
            },
            text: Color::opaque(palette.foreground),
            accent: Color::opaque(palette.accent),
        }
    }
}

/// One key cap.
pub fn cap(key: impl Into<SharedString>, looks: &Looks) -> Div {
    div()
        .flex_none()
        .px(px(5.0))
        .rounded(px(4.0))
        .border_1()
        .border_color(rgba(looks.outline))
        .bg(rgba(looks.cap_fill))
        .text_color(rgba(looks.cap_text))
        .text_size(px(12.0))
        .child(key.into())
}

/// A shortcut's caps followed by what it does: `[Enter] Close`.
pub fn hint(keys: &[&'static str], action: &'static str, looks: &Looks) -> Div {
    div()
        .flex_none()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(3.0))
        .children(keys.iter().map(|key| cap(*key, looks)))
        .child(div().ml(px(3.0)).text_color(rgba(looks.hint)).child(action))
}

/// A row of hints, spaced apart, wrapping when narrow.
pub fn hints(hints: impl IntoIterator<Item = Div>) -> Div {
    div()
        .flex()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap_x(px(12.0))
        .gap_y(px(4.0))
        .children(hints)
}

/// A text button; the caller adds what a click does.
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    looks: &Looks,
) -> Stateful<Div> {
    let hover = looks.button_hover;
    div()
        .id(id)
        .flex_none()
        .px(px(10.0))
        .py(px(2.0))
        .rounded(px(5.0))
        .border_1()
        .border_color(rgba(looks.outline))
        .cursor_pointer()
        .bg(rgba(looks.button))
        .text_color(rgba(looks.text))
        .hover(move |style| style.bg(rgba(hover)))
        .child(label.into())
}

/// An icon from the bundled symbols font, in `color`.
pub fn glyph(text: &'static str, color: Color) -> Div {
    div()
        .flex_none()
        .w(px(16.0))
        .font_family(fonts::SYMBOLS_FAMILY)
        .text_color(rgba(color))
        .child(text)
}
