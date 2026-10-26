//! What the window draws around the panes, worked out from the workspace
//! (M1 spec §7.2): the title, the top bar's session chip and tab labels, and
//! each visible pane's rounded box. Pure, so the GPUI view only paints it.

use shoal_config::{
    Palette,
    schema::{Rgb, WindowConfig},
};
use shoal_model::{Divider, PaneId, Rect, SessionId, Tab, TabId, Workspace, dividers, rects};

/// The top bar's height in logical pixels.
pub const TOP_BAR_HEIGHT: u32 = 34;
/// The active tab's underline.
pub const UNDERLINE_WIDTH: f32 = 2.0;
const FOCUS_BORDER_WIDTH: f32 = 1.5;
const BORDER_WIDTH: f32 = 1.0;
/// Unfocused pane borders are `lighter_background` at this alpha.
const BORDER_ALPHA: f32 = 0.6;

/// A theme color with the alpha it is drawn at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Color {
    pub rgb: Rgb,
    pub alpha: f32,
}

impl Color {
    fn opaque(rgb: Rgb) -> Self {
        Self { rgb, alpha: 1.0 }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Chrome {
    /// `<session> — Shoal`.
    pub title: String,
    pub bar: Color,
    pub chip: Chip,
    pub tabs: Vec<TabLabel>,
    /// The panes the active tab shows, in layout order.
    pub panes: Vec<PaneBox>,
    /// The gaps between them that drag, parent splits first; none while a
    /// pane is zoomed.
    pub dividers: Vec<Divider>,
}

/// The session name at the left of the top bar.
#[derive(Debug, Clone, PartialEq)]
pub struct Chip {
    pub name: String,
    pub fill: Color,
    pub text: Color,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TabLabel {
    pub id: TabId,
    pub glyph: Glyph,
    pub title: String,
    pub text: Color,
    /// The active tab's underline.
    pub underline: Option<Color>,
    /// A pane in the tab rang its bell.
    pub bell: Option<Color>,
}

/// The icon before a tab's title, from the bundled symbols font.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Glyph {
    Terminal,
    /// The tab shows one pane zoomed.
    Zoomed,
}

impl Glyph {
    pub fn text(self) -> &'static str {
        match self {
            // Octicons `terminal` and `screen-full`.
            Self::Terminal => "\u{f489}",
            Self::Zoomed => "\u{f50c}",
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
    pub border: Color,
    pub border_width: f32,
}

/// The chrome for `session` in a window whose pane area (the window below
/// the top bar) is `width` × `height` logical pixels. `None` once the
/// session is gone.
pub fn chrome(
    workspace: &Workspace,
    session: SessionId,
    palette: &Palette,
    window: &WindowConfig,
    width: u32,
    height: u32,
) -> Option<Chrome> {
    let session = workspace.sessions.iter().find(|s| s.id == session)?;
    let opacity = window.opacity as f32;
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
                    palette.foreground
                } else {
                    palette.inactive_text
                }),
                underline: active.then_some(Color::opaque(palette.accent)),
                bell: bell.then_some(Color::opaque(palette.accent)),
            }
        })
        .collect();
    let (panes, dividers) = session
        .tabs
        .iter()
        .find(|tab| tab.id == session.active_tab)
        .map(|tab| pane_boxes(tab, palette, window, width, height))
        .unwrap_or_default();
    Some(Chrome {
        title: format!("{} — Shoal", session.name),
        bar: translucent(palette.top_bar),
        chip: Chip {
            name: session.name.clone(),
            fill: translucent(palette.unfocused_border),
            text: Color::opaque(palette.foreground),
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
pub fn pane_at(panes: &[PaneBox], (x, y): (f32, f32)) -> Option<PaneId> {
    let within = |start: u32, extent: u32, at: f32| {
        (start as f32..start as f32 + extent as f32).contains(&at)
    };
    panes
        .iter()
        .find(|pane| {
            within(pane.rect.x, pane.rect.width, x) && within(pane.rect.y, pane.rect.height, y)
        })
        .map(|pane| pane.pane)
}

fn pane_boxes(
    tab: &Tab,
    palette: &Palette,
    window: &WindowConfig,
    width: u32,
    height: u32,
) -> (Vec<PaneBox>, Vec<Divider>) {
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
                    alpha: window.opacity as f32,
                },
                border: if focused {
                    Color::opaque(palette.accent)
                } else {
                    Color {
                        rgb: palette.unfocused_border,
                        alpha: BORDER_ALPHA,
                    }
                },
                border_width: if focused {
                    FOCUS_BORDER_WIDTH
                } else {
                    BORDER_WIDTH
                },
            }
        })
        .collect();
    (boxes, gaps)
}

#[cfg(test)]
#[path = "chrome_tests.rs"]
mod tests;
