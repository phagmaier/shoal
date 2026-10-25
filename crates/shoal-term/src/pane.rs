//! The daemon's pane terminal (M1 spec §4.2): the one terminal that answers
//! program queries and reports what the pane's metadata follows (§4.3).

use std::{cell::RefCell, rc::Rc};

use libghostty_vt::{
    style::{Palette, RgbColor},
    terminal::{
        ColorScheme, ConformanceLevel, DeviceAttributeFeature, DeviceAttributes, DeviceType,
        Mode as GhosttyMode, PrimaryDeviceAttributes, SecondaryDeviceAttributes,
        SemanticPromptKind, SizeReportSize, TertiaryDeviceAttributes,
    },
};

use crate::{ReplySink, Rgb, TermError, Terminal};

/// Default colors: what SGR defaults draw and what programs get when they ask
/// for colors (OSC 4, 10, 11 and 12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Rgb,
    pub ansi: [Rgb; 16],
}

/// Something a program told the pane terminal that pane metadata follows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TermEvent {
    Bell,
    /// An OSC 0 or 2 title, as raw bytes.
    Title(Vec<u8>),
    /// An OSC 7 directory report, as raw bytes.
    Directory(Vec<u8>),
    Prompt(PromptMark),
}

/// An OSC 133 shell-integration mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptMark {
    PromptStart,
    InputStart,
    OutputStart,
    CommandEnd { exit_code: Option<i32> },
}

/// Where the pane terminal's [`TermEvent`]s go.
pub type EventSink = Box<dyn FnMut(TermEvent)>;

/// What a new pane terminal is set up with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneSetup {
    pub cols: u16,
    pub rows: u16,
    pub scrollback: u32,
    pub theme: Theme,
    /// The XTVERSION answer, such as `shoal 0.1.0`.
    pub version: &'static str,
}

// Ghostty's own answers: a VT220 with ANSI color (`CSI ? 62 ; 22 c`), then
// `CSI > 1 ; 0 ; 0 c` and unit id 0.
const DEVICE_ATTRIBUTES: DeviceAttributes = DeviceAttributes {
    primary: PrimaryDeviceAttributes::new(
        ConformanceLevel::VT220,
        &[DeviceAttributeFeature::ANSI_COLOR],
    ),
    secondary: SecondaryDeviceAttributes {
        device_type: DeviceType::VT220,
        firmware_version: 0,
        rom_cartridge: 0,
    },
    tertiary: TertiaryDeviceAttributes { unit_id: 0 },
};

impl Terminal {
    /// The daemon's pane terminal: replies go to `replies` (the PTY), metadata
    /// to `events`. Grapheme clustering (mode 2027) is on, including after a
    /// full reset, and a program can still turn it off.
    pub fn pane(
        setup: PaneSetup,
        replies: ReplySink,
        events: EventSink,
    ) -> Result<Self, TermError> {
        let mut terminal = Self::with_replies(setup.cols, setup.rows, setup.scrollback, replies)?;
        terminal.set_theme(&setup.theme)?;
        let version = setup.version;
        let inner = &mut terminal.inner;
        inner
            .set_default_mode(GhosttyMode::GRAPHEME_CLUSTER, true)?
            .set_terminfo_name("xterm-256color")?
            .on_device_attributes(|_| Some(DEVICE_ATTRIBUTES))?
            .on_xtversion(move |_| Some(version))?
            // Shoal's protocol has cells only, so pixel sizes report zero.
            .on_size(|term| {
                Some(SizeReportSize {
                    rows: term.rows().ok()?,
                    columns: term.cols().ok()?,
                    cell_width: 0,
                    cell_height: 0,
                })
            })?
            .on_color_scheme(|term| {
                let background = term.default_bg_color().ok()??;
                Some(if is_dark(background) {
                    ColorScheme::Dark
                } else {
                    ColorScheme::Light
                })
            })?;
        // One sink, shared by four callbacks that never run at the same time.
        let events = Rc::new(RefCell::new(events));
        let sink = events.clone();
        inner.on_bell(move |_| emit(&sink, TermEvent::Bell))?;
        let sink = events.clone();
        inner.on_title_changed(move |term| {
            if let Ok(title) = term.title() {
                emit(&sink, TermEvent::Title(title.to_vec()));
            }
        })?;
        let sink = events.clone();
        inner.on_pwd_changed(move |term| {
            if let Ok(pwd) = term.pwd() {
                emit(&sink, TermEvent::Directory(pwd.to_vec()));
            }
        })?;
        inner.on_semantic_prompt(move |_, prompt| {
            let mark = match prompt.kind() {
                Ok(SemanticPromptKind::PromptStart) => PromptMark::PromptStart,
                Ok(SemanticPromptKind::InputStart) => PromptMark::InputStart,
                Ok(SemanticPromptKind::OutputStart) => PromptMark::OutputStart,
                Ok(SemanticPromptKind::CommandEnd) => PromptMark::CommandEnd {
                    exit_code: prompt.exit_code(),
                },
                // A kind from a newer libghostty.
                _ => return,
            };
            emit(&events, TermEvent::Prompt(mark));
        })?;
        Ok(terminal)
    }

    /// Replace the default colors. Colors a program set itself (OSC 4, 10–12)
    /// stay until it resets them.
    pub fn set_theme(&mut self, theme: &Theme) -> Result<(), TermError> {
        let mut palette = self.inner.default_color_palette()?;
        for (slot, color) in palette.0.iter_mut().zip(theme.ansi) {
            *slot = rgb(color);
        }
        self.inner
            .set_default_fg_color(Some(rgb(theme.foreground)))?
            .set_default_bg_color(Some(rgb(theme.background)))?
            .set_default_cursor_color(Some(rgb(theme.cursor)))?
            .set_default_color_palette(Some(Palette(palette.0)))?;
        Ok(())
    }
}

fn emit(sink: &RefCell<EventSink>, event: TermEvent) {
    (sink.borrow_mut())(event);
}

fn rgb(Rgb { r, g, b }: Rgb) -> RgbColor {
    RgbColor { r, g, b }
}

/// Relative luminance below the midpoint (Rec. 709 weights, sRGB as is).
fn is_dark(RgbColor { r, g, b }: RgbColor) -> bool {
    2126 * u32::from(r) + 7152 * u32::from(g) + 722 * u32::from(b) < 128 * 10_000
}
