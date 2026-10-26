//! The font chain and glyph cache (M1 spec §7.3). Glyphs are found once per
//! font face and grapheme, then placed cell by cell; the shaper never
//! decides positions or widths, so there are no ligatures across cells. A
//! multi-codepoint grapheme cluster is shaped as one unit.

use std::{collections::HashMap, rc::Rc};

use gpui::{
    Font, FontFallbacks, FontFeatures, FontId, FontStyle, FontWeight, GlyphId, Hsla, Pixels,
    SharedString, TextRun, WindowTextSystem, px,
};

use crate::{draw::Face, fonts::SYMBOLS_FAMILY, metrics::FontMeasure};

/// The system emoji font, after the bundled symbols font.
const EMOJI_FAMILY: &str = "Noto Color Emoji";

/// `font.family`, then the bundled symbols, then emoji; the platform falls
/// back to any system font after that.
pub fn font(family: &str, face: Face) -> Font {
    Font {
        family: SharedString::from(family.to_owned()),
        features: FontFeatures::default(),
        fallbacks: Some(FontFallbacks::from_fonts(vec![
            SYMBOLS_FAMILY.to_owned(),
            EMOJI_FAMILY.to_owned(),
        ])),
        weight: if face.bold {
            FontWeight::BOLD
        } else {
            FontWeight::NORMAL
        },
        style: if face.italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        },
    }
}

/// Measure the regular face at `size` for the cell metrics.
pub fn measure(family: &str, size: f32, text: &WindowTextSystem) -> FontMeasure {
    let id = text.resolve_font(&font(family, Face::default()));
    let size = px(size);
    let advance = text
        .advance(id, size, 'M')
        .map(|advance| f32::from(advance.width))
        .unwrap_or(f32::from(size) * 0.6);
    FontMeasure {
        advance,
        ascent: f32::from(text.ascent(id, size)),
        descent: f32::from(text.descent(id, size)).abs(),
    }
}

/// One glyph of a cell's text, offset from the cell's left edge.
#[derive(Debug, Clone, Copy)]
pub struct Placed {
    pub font: FontId,
    pub glyph: GlyphId,
    pub x: Pixels,
    pub emoji: bool,
}

/// What a grapheme draws: its glyphs and their total advance.
#[derive(Debug, Clone)]
pub struct Shaped {
    pub glyphs: Rc<[Placed]>,
    pub width: Pixels,
}

/// The fonts a cache was filled for; any change empties it.
#[derive(Debug, Clone, PartialEq)]
struct Key {
    family: String,
    size: f32,
}

/// Glyphs by face and grapheme, for one font family and size.
#[derive(Debug, Default)]
pub struct GlyphCache {
    key: Option<Key>,
    /// Changes whenever the cache empties, so cached rows know their
    /// glyphs are stale.
    generation: u64,
    /// ASCII by `char * 4 + face`, the common case without hashing.
    ascii: Vec<Option<Shaped>>,
    chars: HashMap<(Face, char), Shaped>,
    clusters: HashMap<(Face, String), Shaped>,
}

fn face_index(face: Face) -> usize {
    usize::from(face.bold) * 2 + usize::from(face.italic)
}

impl GlyphCache {
    /// Empty the cache unless it was filled for this family and size.
    pub fn use_font(&mut self, family: &str, size: f32) {
        let key = Key {
            family: family.to_owned(),
            size,
        };
        if self.key.as_ref() != Some(&key) {
            *self = Self {
                key: Some(key),
                generation: self.generation + 1,
                ..Self::default()
            };
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The glyphs for `text` (one cell's grapheme) in `face`.
    pub fn get(&mut self, text: &str, face: Face, text_system: &WindowTextSystem) -> Shaped {
        let mut chars = text.chars();
        let single = match (chars.next(), chars.next()) {
            (Some(ch), None) => Some(ch),
            _ => None,
        };
        match single {
            Some(ch) if ch.is_ascii() => {
                let slot = ch as usize * 4 + face_index(face);
                if self.ascii.is_empty() {
                    self.ascii = vec![None; 128 * 4];
                }
                if let Some(shaped) = &self.ascii[slot] {
                    return shaped.clone();
                }
                let shaped = self.shape(text, face, text_system);
                self.ascii[slot] = Some(shaped.clone());
                shaped
            }
            Some(ch) => {
                if let Some(shaped) = self.chars.get(&(face, ch)) {
                    return shaped.clone();
                }
                let shaped = self.shape(text, face, text_system);
                self.chars.insert((face, ch), shaped.clone());
                shaped
            }
            None => {
                if let Some(shaped) = self.clusters.get(&(face, text.to_owned())) {
                    return shaped.clone();
                }
                let shaped = self.shape(text, face, text_system);
                self.clusters
                    .insert((face, text.to_owned()), shaped.clone());
                shaped
            }
        }
    }

    fn shape(&mut self, text: &str, face: Face, text_system: &WindowTextSystem) -> Shaped {
        let Some(Key { family, size }) = &self.key else {
            return Shaped {
                glyphs: Rc::from([]),
                width: px(0.0),
            };
        };
        let layout = text_system.layout_line(
            text,
            px(*size),
            &[TextRun {
                len: text.len(),
                font: font(family, face),
                color: Hsla::default(),
                background_color: None,
                underline: None,
                strikethrough: None,
            }],
            None,
        );
        let glyphs = layout
            .runs
            .iter()
            .flat_map(|run| {
                run.glyphs.iter().map(|glyph| Placed {
                    font: run.font_id,
                    glyph: glyph.id,
                    x: glyph.position.x,
                    emoji: glyph.is_emoji,
                })
            })
            .collect();
        Shaped {
            glyphs,
            width: layout.width,
        }
    }
}
