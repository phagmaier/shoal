use std::{collections::BTreeMap, str::FromStr};

use serde::{Deserialize, Deserializer, de::Error as _};

use crate::Keymap;

#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Config {
    pub font: FontConfig,
    pub window: WindowConfig,
    pub theme: ThemeConfig,
    pub cursor: CursorConfig,
    pub scrollback: ScrollbackConfig,
    pub shell: ShellConfig,
    pub prompt: PromptConfig,
    pub selection: SelectionConfig,
    pub tabs: TabsConfig,
    pub behavior: BehaviorConfig,
    pub keybinds: Keymap,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct FontConfig {
    pub family: String,
    #[serde(deserialize_with = "positive")]
    pub size: f64,
    #[serde(deserialize_with = "positive")]
    pub line_height: f64,
}
impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: "JetBrains Mono".into(),
            size: 11.0,
            line_height: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct WindowConfig {
    #[serde(deserialize_with = "opacity")]
    pub opacity: f64,
    pub padding: u32,
    pub gap: u32,
    pub radius: u32,
}
impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            opacity: 0.88,
            padding: 8,
            gap: 6,
            radius: 10,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ThemeConfig {
    /// `omarchy` or a path to a colors.toml-style file. Resolved by theme loading.
    pub source: String,
    pub colors: BTreeMap<String, Rgb>,
}
impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            source: "omarchy".into(),
            colors: BTreeMap::new(),
        }
    }
}

macro_rules! config_enum {
    ($name:ident { $default:ident $(, $variant:ident)* $(,)? }) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum $name { #[default] $default, $($variant),* }
    };
}
config_enum!(CursorStyle {
    Block,
    Bar,
    Underline
});
config_enum!(ShellIntegration { Auto, Manual, Off });
config_enum!(PromptMode { Native, Shell });
config_enum!(PromptStyle { Powerline, Lean });
config_enum!(PromptSegment {
    User,
    Path,
    Git,
    Runtime
});
config_enum!(TabTitle { Auto, Program });

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct CursorConfig {
    pub style: CursorStyle,
    pub blink: bool,
    #[serde(deserialize_with = "cursor_color")]
    pub color: String,
}
impl Default for CursorConfig {
    fn default() -> Self {
        Self {
            style: CursorStyle::Block,
            blink: false,
            color: "accent".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ScrollbackConfig {
    #[serde(deserialize_with = "scrollback_lines")]
    pub lines: u32,
    pub persist: bool,
}
impl Default for ScrollbackConfig {
    fn default() -> Self {
        Self {
            lines: 10000,
            persist: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ShellConfig {
    pub program: String,
    pub integration: ShellIntegration,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct PromptConfig {
    pub mode: PromptMode,
    pub style: PromptStyle,
    #[serde(deserialize_with = "segments")]
    pub segments: Vec<PromptSegment>,
}
impl Default for PromptConfig {
    fn default() -> Self {
        Self {
            mode: PromptMode::Native,
            style: PromptStyle::Powerline,
            segments: vec![
                PromptSegment::User,
                PromptSegment::Path,
                PromptSegment::Git,
                PromptSegment::Runtime,
            ],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct SelectionConfig {
    pub copy_on_select: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct TabsConfig {
    pub title: TabTitle,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct BehaviorConfig {
    pub close_pane_on_exit: bool,
    pub confirm_close_running: bool,
}
impl Default for BehaviorConfig {
    fn default() -> Self {
        Self {
            close_pane_on_exit: true,
            confirm_close_running: true,
        }
    }
}

/// Palette keys used by M1 §9.2; no theme lookup or palette mapping occurs here.
pub const PALETTE_KEYS: &[&str] = &[
    "background",
    "foreground",
    "accent",
    "selection",
    "bright_foreground",
    "muted",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "bright_red",
    "bright_green",
    "bright_yellow",
    "bright_blue",
    "bright_magenta",
    "bright_cyan",
    "darker_background",
    "lighter_background",
    "dark_foreground",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct Rgb(pub [u8; 3]);

#[derive(Debug, thiserror::Error)]
#[error("color must be #rrggbb")]
pub struct ColorError;

impl FromStr for Rgb {
    type Err = ColorError;
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        if input.len() != 7
            || !input.starts_with('#')
            || !input.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
        {
            return Err(ColorError);
        }
        let mut bytes = [0; 3];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&input[1 + index * 2..3 + index * 2], 16)
                .map_err(|_| ColorError)?;
        }
        Ok(Self(bytes))
    }
}
impl TryFrom<String> for Rgb {
    type Error = ColorError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

fn positive<'de, D: Deserializer<'de>>(de: D) -> Result<f64, D::Error> {
    let value = f64::deserialize(de)?;
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(D::Error::custom("value must be finite and positive"))
    }
}

fn opacity<'de, D: Deserializer<'de>>(de: D) -> Result<f64, D::Error> {
    let value = f64::deserialize(de)?;
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(value)
    } else {
        Err(D::Error::custom("opacity must be finite and in 0..=1"))
    }
}

fn scrollback_lines<'de, D: Deserializer<'de>>(de: D) -> Result<u32, D::Error> {
    let value = u32::deserialize(de)?;
    if value <= 100000 {
        Ok(value)
    } else {
        Err(D::Error::custom("scrollback lines must be in 0..=100000"))
    }
}

fn cursor_color<'de, D: Deserializer<'de>>(de: D) -> Result<String, D::Error> {
    let value = String::deserialize(de)?;
    if PALETTE_KEYS.contains(&value.as_str()) || value.parse::<Rgb>().is_ok() {
        Ok(value)
    } else {
        Err(D::Error::custom(
            "cursor color must be a palette key or #rrggbb",
        ))
    }
}

fn segments<'de, D: Deserializer<'de>>(de: D) -> Result<Vec<PromptSegment>, D::Error> {
    let value = Vec::<PromptSegment>::deserialize(de)?;
    if value
        .iter()
        .filter(|&&segment| segment == PromptSegment::Path)
        .count()
        == 1
    {
        Ok(value)
    } else {
        Err(D::Error::custom(
            "prompt segments must contain path exactly once",
        ))
    }
}
