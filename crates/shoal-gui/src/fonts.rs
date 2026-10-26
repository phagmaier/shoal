//! The bundled Symbols Nerd Font (M1 spec §7.3): icons, powerline and box
//! drawing glyphs with any `font.family`. Provenance and licenses are in
//! `assets/README.md`.

use std::borrow::Cow;

use gpui::App;

/// The family name inside the bundled font file.
pub const SYMBOLS_FAMILY: &str = "Symbols Nerd Font Mono";

static SYMBOLS: &[u8] = include_bytes!("../assets/SymbolsNerdFontMono-Regular.ttf");

/// Make the bundled font available to every window.
pub fn load(cx: &mut App) {
    if let Err(error) = cx.text_system().add_fonts(vec![Cow::Borrowed(SYMBOLS)]) {
        tracing::warn!(%error, "can't load the bundled symbols font");
    }
}
