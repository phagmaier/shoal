use std::collections::BTreeMap;

use crate::schema::{CursorConfig, PALETTE_KEYS, Rgb};

/// Colors from one source, keyed by canonical palette name or by one of the
/// derivation inputs in [`INPUTS`].
pub(crate) type Layer = BTreeMap<&'static str, Rgb>;

// Not drawn directly; they only feed Omarchy's fallbacks for missing keys.
const INPUTS: &[&str] = &["color8", "selection_background"];

// Omarchy's legacy names in its own resolution order: an earlier alias beats a
// later one for the same key, and the canonical name beats them all.
const ALIASES: &[(&str, &str)] = &[
    ("bg", "background"),
    ("darker_bg", "darker_background"),
    ("lighter_bg", "lighter_background"),
    ("fg", "foreground"),
    ("dark_fg", "dark_foreground"),
    ("bright_fg", "bright_foreground"),
    ("color0", "background"),
    ("color7", "foreground"),
    ("color1", "red"),
    ("color2", "green"),
    ("color3", "yellow"),
    ("color4", "blue"),
    ("color5", "magenta"),
    ("color6", "cyan"),
    ("color9", "bright_red"),
    ("color10", "bright_green"),
    ("color11", "bright_yellow"),
    ("color12", "bright_blue"),
    ("color13", "bright_magenta"),
    ("color14", "bright_cyan"),
    ("purple", "magenta"),
    ("bright_purple", "bright_magenta"),
    ("color15", "bright_foreground"),
];

// Omarchy's default theme, Tokyo Night.
const BUILTIN: [(&str, [u8; 3]); 21] = [
    ("background", [0x1a, 0x1b, 0x26]),
    ("foreground", [0xa9, 0xb1, 0xd6]),
    ("accent", [0x7a, 0xa2, 0xf7]),
    ("selection", [0x29, 0x2e, 0x42]),
    ("bright_foreground", [0xc0, 0xca, 0xf5]),
    ("muted", [0x41, 0x48, 0x68]),
    ("red", [0xf7, 0x76, 0x8e]),
    ("green", [0x9e, 0xce, 0x6a]),
    ("yellow", [0xe0, 0xaf, 0x68]),
    ("blue", [0x7a, 0xa2, 0xf7]),
    ("magenta", [0xad, 0x8e, 0xe6]),
    ("cyan", [0x44, 0x9d, 0xab]),
    ("bright_red", [0xff, 0x7a, 0x93]),
    ("bright_green", [0xb9, 0xf2, 0x7c]),
    ("bright_yellow", [0xff, 0x9e, 0x64]),
    ("bright_blue", [0x7d, 0xa6, 0xff]),
    ("bright_magenta", [0xbb, 0x9a, 0xf7]),
    ("bright_cyan", [0x0d, 0xb9, 0xd7]),
    ("darker_background", [0x0e, 0x0e, 0x14]),
    ("lighter_background", [0x24, 0x28, 0x3b]),
    ("dark_foreground", [0x56, 0x5f, 0x89]),
];

/// The key a source key feeds and its rank (lower wins within one source), or
/// `None` when Shoal does not use the key.
pub(crate) fn used_key(key: &str) -> Option<(&'static str, usize)> {
    if let Some(name) = PALETTE_KEYS.iter().chain(INPUTS).find(|name| **name == key) {
        return Some((name, 0));
    }
    ALIASES
        .iter()
        .position(|(alias, _)| *alias == key)
        .map(|index| (ALIASES[index].1, index + 1))
}

/// Build one source's layer from `(key, color)` entries in any order.
pub(crate) fn layer<'a>(entries: impl IntoIterator<Item = (&'a str, Rgb)>) -> Layer {
    let mut ranked = BTreeMap::new();
    for (key, rgb) in entries {
        let Some((name, rank)) = used_key(key) else {
            continue;
        };
        let slot = ranked.entry(name).or_insert((rank, rgb));
        if rank < slot.0 {
            *slot = (rank, rgb);
        }
    }
    ranked
        .into_iter()
        .map(|(name, (_, rgb))| (name, rgb))
        .collect()
}

/// Every palette key resolved, after layering, derivation and built-in fills.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeColors(BTreeMap<&'static str, Rgb>);

impl ThemeColors {
    /// Resolve layers given highest precedence first.
    pub(crate) fn resolve(layers: &[Layer]) -> Self {
        let mut colors = Layer::new();
        for layer in layers {
            for (&name, &rgb) in layer {
                colors.entry(name).or_insert(rgb);
            }
        }
        derive(&mut colors);
        for (name, rgb) in BUILTIN {
            colors.entry(name).or_insert(Rgb(rgb));
        }
        colors.retain(|name, _| PALETTE_KEYS.contains(name));
        Self(colors)
    }

    /// A canonical palette key such as `magenta`; legacy names are not keys.
    pub fn get(&self, key: &str) -> Option<Rgb> {
        self.0.get(key).copied()
    }

    /// The M1 spec §9.2 mapping onto what Shoal draws.
    pub fn palette(&self, cursor: &CursorConfig) -> Palette {
        let key = |name: &str| self.0[name];
        Palette {
            background: key("background"),
            foreground: key("foreground"),
            cursor: self
                .get(&cursor.color)
                .or_else(|| cursor.color.parse().ok())
                .unwrap_or(key("accent")),
            selection_background: key("selection"),
            selection_foreground: key("bright_foreground"),
            ansi: [
                "background",
                "red",
                "green",
                "yellow",
                "blue",
                "magenta",
                "cyan",
                "foreground",
                "muted",
                "bright_red",
                "bright_green",
                "bright_yellow",
                "bright_blue",
                "bright_magenta",
                "bright_cyan",
                "bright_foreground",
            ]
            .map(key),
            top_bar: key("darker_background"),
            accent: key("accent"),
            unfocused_border: key("lighter_background"),
            inactive_text: key("dark_foreground"),
        }
    }
}

/// Colors by role. The background is drawn at `window.opacity`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub background: Rgb,
    pub foreground: Rgb,
    pub cursor: Rgb,
    pub selection_background: Rgb,
    pub selection_foreground: Rgb,
    pub ansi: [Rgb; 16],
    pub top_bar: Rgb,
    /// Focus border, active-tab underline, bell dot and prompt character.
    pub accent: Rgb,
    pub unfocused_border: Rgb,
    /// Tooltip and key-hint text.
    pub inactive_text: Rgb,
}

// The fallbacks in Omarchy's omarchy-theme-color, in its order, for the keys
// Shoal uses. Each fills a key only from colors the theme actually has.
fn derive(colors: &mut Layer) {
    let mut fill = |name: &'static str, from: &[&str]| {
        if !colors.contains_key(name)
            && let Some(rgb) = from.iter().find_map(|key| colors.get(key).copied())
        {
            colors.insert(name, rgb);
        }
    };
    fill("bright_foreground", &["foreground"]);
    fill("lighter_background", &["background"]);
    fill("dark_foreground", &["color8", "foreground"]);
    fill("muted", &["color8", "dark_foreground"]);
    fill(
        "selection",
        &["selection_background", "color8", "background"],
    );
    let mut mix = |name: &'static str, base: &str, toward: u8, percent: u32| {
        if !colors.contains_key(name)
            && let Some(Rgb(start)) = colors.get(base).copied()
        {
            colors.insert(
                name,
                Rgb(start.map(|channel| mix_channel(channel, toward, percent))),
            );
        }
    };
    mix("darker_background", "background", 0x00, 50);
    for base in ["red", "yellow", "green", "cyan", "blue", "magenta"] {
        let bright = PALETTE_KEYS
            .iter()
            .find(|name| name.strip_prefix("bright_") == Some(base))
            .expect("invariant: every ANSI base color has a bright palette key");
        mix(bright, base, 0xff, 20);
    }
}

/// Omarchy's `int(start * (1 - a) + end * a + 0.5)`, in exact integer math.
fn mix_channel(start: u8, end: u8, percent: u32) -> u8 {
    let mixed = (u32::from(start) * (100 - percent) + u32::from(end) * percent + 50) / 100;
    u8::try_from(mixed).expect("invariant: a mix of two channels is a channel")
}
