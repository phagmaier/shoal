//! Just enough of Omarchy's colors.toml for the spike (no toml crate).

use libghostty_vt::style::RgbColor;

pub struct Theme {
    pub background: RgbColor,
    pub foreground: RgbColor,
    pub accent: RgbColor,
    /// ANSI 0-15, mapped as in M1 spec 9.2.
    pub ansi: [RgbColor; 16],
}

fn hex(s: &str) -> Option<RgbColor> {
    let s = s.trim().trim_matches('"').trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some(RgbColor {
        r: (v >> 16) as u8,
        g: (v >> 8) as u8,
        b: v as u8,
    })
}

pub fn load() -> Theme {
    let home = std::env::var("HOME").unwrap_or_default();
    let text = [
        format!("{home}/.local/state/omarchy/current/theme/colors.toml"),
        format!("{home}/.config/omarchy/current/theme/colors.toml"),
    ]
    .iter()
    .find_map(|p| std::fs::read_to_string(p).ok())
    .unwrap_or_default();
    let get = |key: &str, fallback: u32| {
        text.lines()
            .filter_map(|l| l.split_once('='))
            .find(|(k, _)| k.trim() == key)
            .and_then(|(_, v)| hex(v))
            .unwrap_or(RgbColor {
                r: (fallback >> 16) as u8,
                g: (fallback >> 8) as u8,
                b: fallback as u8,
            })
    };
    let background = get("background", 0x1e1e2e);
    let foreground = get("foreground", 0xcdd6f4);
    let bright_foreground = get("bright_foreground", 0xffffff);
    let names = ["red", "green", "yellow", "blue", "magenta", "cyan"];
    let mut ansi = [background; 16];
    ansi[0] = background;
    ansi[8] = get("muted", 0x585b70);
    ansi[7] = foreground;
    ansi[15] = bright_foreground;
    for (i, name) in names.iter().enumerate() {
        ansi[1 + i] = get(name, 0x808080);
        ansi[9 + i] = get(&format!("bright_{name}"), 0x808080);
    }
    Theme {
        background,
        foreground,
        accent: get("accent", 0x89b4fa),
        ansi,
    }
}
