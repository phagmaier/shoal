//! What the window draws around the panes, worked out from the workspace
//! (M1 spec §7.2): the title, the top bar's session chip and tab labels, and
//! each visible pane's rounded box and focus edge. Pure, so the GPUI view
//! only paints it.

use shoal_config::{
    Palette,
    schema::{Rgb, WindowConfig},
};
use shoal_model::{
    Divider, PaneId, PaneStatus, Rect, SessionId, Tab, TabId, Workspace, dividers, rects,
};

/// The top bar's height in logical pixels.
pub const TOP_BAR_HEIGHT: u32 = 34;
/// A retained exited pane's footer, below its grid.
pub const FOOTER_HEIGHT: u32 = 30;
/// Chrome rules are `lighter_background` at this alpha.
const BORDER_ALPHA: f32 = 0.6;
/// The active tab's pill is `accent` at this alpha.
const ACTIVE_TAB_ALPHA: f32 = 0.16;

/// A theme color with the alpha it is drawn at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub rgb: Rgb,
    pub alpha: f32,
}

impl Color {
    pub fn opaque(rgb: Rgb) -> Self {
        Self { rgb, alpha: 1.0 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Chrome {
    /// `<session> — Shoal`.
    pub title: String,
    pub bar: Color,
    pub chip: Chip,
    /// The hairline after the session chip.
    pub rule: Color,
    pub tabs: Vec<TabLabel>,
    /// The panes the active tab shows, in layout order.
    pub panes: Vec<PaneBox>,
    /// The gaps between them that drag, parent splits first; none while a
    /// pane is zoomed.
    pub dividers: Vec<Divider>,
}

/// The session's glyph and name at the left of the top bar.
#[derive(Debug, Clone, PartialEq)]
pub struct Chip {
    pub name: String,
    pub text: Color,
    /// Behind it while the pointer is over it.
    pub hover: Color,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabLabel {
    pub id: TabId,
    pub glyph: Glyph,
    pub title: String,
    pub text: Color,
    /// The active tab's pill.
    pub fill: Option<Color>,
    /// A pane in the tab rang its bell.
    pub bell: Option<Color>,
}

/// A chrome icon from the bundled symbols font.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Terminal,
    /// A zoomed tab.
    Zoomed,
    Session,
}

impl Glyph {
    pub fn text(self) -> &'static str {
        match self {
            // Octicons `terminal` and `screen-full`, Codicons `layers`.
            Self::Terminal => "\u{f489}",
            Self::Zoomed => "\u{f50c}",
            Self::Session => "\u{ebd2}",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PaneBox {
    pub pane: PaneId,
    /// In pane-area pixels: the area below the top bar.
    pub rect: Rect,
    pub radius: f32,
    pub fill: Color,
    /// A soft line along the top edge; only the focused pane of a split.
    pub focus: Option<Color>,
    /// A faint outline accompanying the focused pane's top line.
    pub edge: Option<Color>,
    /// Height reserved below the grid for an exited pane's footer.
    pub footer: u32,
}

impl PaneBox {
    /// The grid's content rectangle, excluding only an exited footer.
    /// Grid sizing, painting, pointer mapping and IME all share this area.
    pub fn content(&self) -> Rect {
        Rect {
            height: self.rect.height.saturating_sub(self.footer),
            ..self.rect
        }
    }
}

/// Two layers with the requested combined opacity. Above 0.5, half the
/// remaining transmission belongs to each layer. At 1 both are opaque.
/// Below 0.5 the pane keeps all the tint, leaving the glass transparent.
pub fn surface_alphas(opacity: f64) -> (f32, f32) {
    let opacity = opacity as f32;
    let glass = (2.0 * opacity - 1.0).max(0.0);
    let pane = if glass < 1.0 {
        (opacity - glass) / (1.0 - glass)
    } else {
        1.0
    };
    (glass, pane)
}

/// The window-wide tint, including the gaps and padding.
pub fn glass(palette: &Palette, window: &WindowConfig) -> Color {
    Color {
        rgb: palette.top_bar,
        alpha: surface_alphas(window.opacity).0,
    }
}

/// The chrome for `session` in a window whose pane area (the window below
/// the top bar) is `width` × `height` logical pixels. `None` once the
/// session is gone.
pub fn chrome(
    workspace: &Workspace,
    session: SessionId,
    palette: &Palette,
    window: &WindowConfig,
    (width, height): (u32, u32),
) -> Option<Chrome> {
    let session = workspace.sessions.iter().find(|s| s.id == session)?;
    let opacity = surface_alphas(window.opacity).1;
    let translucent = |rgb| Color {
        rgb,
        alpha: opacity,
    };
    let tabs = session
        .tabs
        .iter()
        .map(|tab| {
            let active = tab.id == session.active_tab;
            let title = tab_title(workspace, tab);
            let mut bell = false;
            tab.layout.for_each_pane(&mut |pane| {
                bell |= workspace.panes.get(&pane).is_some_and(|pane| pane.bell);
            });
            TabLabel {
                id: tab.id,
                glyph: if tab.zoomed.is_some() {
                    Glyph::Zoomed
                } else {
                    Glyph::Terminal
                },
                title,
                text: Color::opaque(if active {
                    palette.accent
                } else {
                    palette.inactive_text
                }),
                fill: active.then_some(Color {
                    rgb: palette.accent,
                    alpha: ACTIVE_TAB_ALPHA,
                }),
                bell: bell.then_some(Color::opaque(palette.accent)),
            }
        })
        .collect();
    let looks = Looks {
        workspace,
        palette,
        window,
    };
    let (panes, dividers) = session
        .tabs
        .iter()
        .find(|tab| tab.id == session.active_tab)
        .map(|tab| looks.pane_boxes(tab, width, height))
        .unwrap_or_default();
    Some(Chrome {
        title: format!("{} — Shoal", session.name),
        bar: translucent(palette.top_bar),
        chip: Chip {
            name: session.name.clone(),
            text: Color::opaque(palette.accent),
            hover: translucent(palette.unfocused_border),
        },
        rule: Color {
            rgb: palette.unfocused_border,
            alpha: BORDER_ALPHA,
        },
        tabs,
        panes,
        dividers,
    })
}

/// A tab's label: its name, otherwise its focused pane's title.
pub fn tab_title(workspace: &Workspace, tab: &Tab) -> String {
    tab.name.clone().unwrap_or_else(|| {
        workspace
            .panes
            .get(&tab.focused)
            .map(|pane| pane.title.clone())
            .unwrap_or_default()
    })
}

/// The pane whose box holds `point`, in pane-area pixels.
pub fn pane_at(panes: &[PaneBox], point: (f32, f32)) -> Option<PaneId> {
    box_at(panes, point).map(|pane| pane.pane)
}

/// The box that holds `point`, in pane-area pixels.
pub fn box_at(panes: &[PaneBox], (x, y): (f32, f32)) -> Option<&PaneBox> {
    let within = |start: u32, extent: u32, at: f32| {
        (start as f32..start as f32 + extent as f32).contains(&at)
    };
    panes.iter().find(|pane| {
        within(pane.rect.x, pane.rect.width, x) && within(pane.rect.y, pane.rect.height, y)
    })
}

/// What the pane boxes are drawn from.
struct Looks<'a> {
    workspace: &'a Workspace,
    palette: &'a Palette,
    window: &'a WindowConfig,
}

impl Looks<'_> {
    fn pane_boxes(&self, tab: &Tab, width: u32, height: u32) -> (Vec<PaneBox>, Vec<Divider>) {
        let (palette, window) = (self.palette, self.window);
        let padding = window.padding;
        let area = Rect {
            x: padding.min(width),
            y: padding.min(height),
            width: width.saturating_sub(padding.saturating_mul(2)),
            height: height.saturating_sub(padding.saturating_mul(2)),
        };
        let (shown, gaps): (Vec<(PaneId, Rect)>, _) = match tab.zoomed {
            Some(zoomed) => (vec![(zoomed, area)], Vec::new()),
            None => match rects(&tab.layout, area, window.gap)
                .and_then(|rects| Ok((rects, dividers(&tab.layout, area, window.gap)?)))
            {
                Ok((rects, gaps)) => {
                    let mut order = Vec::new();
                    tab.layout.for_each_pane(&mut |pane| order.push(pane));
                    let shown = order
                        .into_iter()
                        .filter_map(|pane| Some((pane, *rects.get(&pane)?)))
                        .collect();
                    (shown, gaps)
                }
                Err(error) => {
                    tracing::warn!(%error, "can't lay out the tab");
                    (Vec::new(), Vec::new())
                }
            },
        };
        // With one pane in view, which one has focus goes without saying.
        let marks_focus = shown.len() > 1;
        let boxes = shown
            .into_iter()
            .map(|(pane, rect)| {
                let focused = marks_focus && pane == tab.focused;
                PaneBox {
                    pane,
                    rect,
                    radius: window.radius as f32,
                    fill: Color {
                        rgb: palette.background,
                        alpha: surface_alphas(window.opacity).1,
                    },
                    focus: focused.then_some(Color {
                        rgb: palette.accent,
                        alpha: 0.70,
                    }),
                    edge: focused.then_some(Color {
                        rgb: palette.accent,
                        alpha: 0.34,
                    }),
                    footer: match self.workspace.panes.get(&pane).map(|p| p.status) {
                        Some(PaneStatus::Exited { .. }) => FOOTER_HEIGHT,
                        _ => 0,
                    },
                }
            })
            .collect();
        (boxes, gaps)
    }
}

#[cfg(test)]
#[path = "chrome_tests.rs"]
mod tests;
