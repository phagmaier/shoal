//! The window's root view: the top bar and pane boxes from [`chrome`],
//! each pane's mirror drawn inside its box, following the workspace as the
//! daemon changes it.

use std::time::Instant;

use gpui::{
    AnyElement, BorderStyle, Bounds, ContentMask, Context, Div, Hsla, Rgba, SharedString,
    Subscription, Window, canvas, div, point, prelude::*, px, quad, size,
};
use shoal_client::{
    Client, Event,
    model::{PaneId, SessionId, Tab, Workspace},
    proto::Notification,
};
use shoal_config::{Config, Palette};

use crate::{
    chrome::{self, Chrome, Color, PaneBox, TOP_BAR_HEIGHT, TabLabel, UNDERLINE_WIDTH},
    draw, fonts,
    glyphs::{self, GlyphCache},
    metrics::{self, CellMetrics},
    mirror::{self, State},
    panes::Panes,
    session,
    terminal::{self, PaneFrame},
};

/// Chrome text size in logical pixels.
const TEXT_SIZE: f32 = 13.0;
const GLYPH_WIDTH: f32 = 16.0;

pub struct ShoalWindow {
    pub(crate) client: Client,
    session: SessionId,
    workspace: Workspace,
    config: Config,
    palette: Palette,
    title: String,
    pub(crate) panes: Panes,
    glyphs: GlyphCache,
    /// Cell metrics, with the font family and line height they were
    /// measured for (the size and scale are in the metrics).
    metrics: Option<(CellMetrics, String, f64)>,
    _activation: Subscription,
}

impl ShoalWindow {
    pub fn new(
        client: Client,
        session: SessionId,
        workspace: Workspace,
        config: Config,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // An inactive window draws its cursors hollow.
        let activation = cx.observe_window_activation(window, |_, _, cx| cx.notify());
        let mut view = Self {
            client,
            session,
            workspace,
            panes: Panes::new(mirror::Setup::from_config(&config)),
            config,
            palette,
            title: String::new(),
            glyphs: GlyphCache::default(),
            metrics: None,
            _activation: activation,
        };
        view.sync_panes(cx);
        view
    }

    fn sync_panes(&mut self, cx: &mut Context<Self>) {
        self.panes
            .sync(&self.workspace, self.session, &self.client, cx);
    }

    /// Apply one connection event. Returns `false` once the session is gone
    /// and the window should close.
    pub fn on_event(&mut self, event: Event, cx: &mut Context<Self>) -> bool {
        match event {
            Event::Notification(Notification::WorkspaceChanged(state)) => {
                // Always the newest: one connection's notifications arrive in
                // order, and a restarted daemon may restore an older `rev`.
                self.workspace = state.workspace;
                self.sync_panes(cx);
                cx.notify();
                return self.shows_session();
            }
            Event::Notification(Notification::DaemonNotice { level, message, .. }) => {
                // T46 shows these as banners.
                tracing::info!(?level, %message, "daemon notice");
            }
            Event::Notification(other) => tracing::debug!(?other, "ignored notification"),
            Event::Connected(_) => {
                // The daemon forgot this window with the old connection. The
                // answer doesn't matter here: a closed session arrives as a
                // workspace change.
                drop(self.client.send(session::show_call(self.session)));
            }
            Event::Disconnected(error) => {
                // T46 shows the reconnecting overlay.
                tracing::warn!(%error, "lost shoald; reconnecting");
            }
        }
        true
    }

    fn shows_session(&self) -> bool {
        self.workspace.sessions.iter().any(|s| s.id == self.session)
    }

    fn active_tab(&self) -> Option<&Tab> {
        let session = self
            .workspace
            .sessions
            .iter()
            .find(|s| s.id == self.session)?;
        session.tabs.iter().find(|tab| tab.id == session.active_tab)
    }

    /// Whether `pane` is on screen: in the active tab, or its zoomed pane.
    pub(crate) fn shows(&self, pane: PaneId) -> bool {
        let Some(tab) = self.active_tab() else {
            return false;
        };
        if let Some(zoomed) = tab.zoomed {
            return zoomed == pane;
        }
        let mut found = false;
        tab.layout
            .for_each_pane(&mut |shown| found |= shown == pane);
        found
    }

    /// The pane the active tab focuses, which gets keyboard input.
    fn focused_pane(&self) -> Option<PaneId> {
        self.active_tab().map(|tab| tab.focused)
    }

    /// Cell metrics for the configured font at the window's scale,
    /// measured again only when either changes.
    fn cell_metrics(&mut self, window: &Window) -> CellMetrics {
        let scale = window.scale_factor();
        let font = &self.config.font;
        let size = metrics::font_size(font.size);
        self.glyphs.use_font(&font.family, size);
        match &self.metrics {
            Some((cached, family, line_height))
                if cached.scale == scale
                    && cached.font_size == size
                    && *family == font.family
                    && *line_height == font.line_height =>
            {
                *cached
            }
            _ => {
                let measure = glyphs::measure(&font.family, size, window.text_system());
                let cell = metrics::cell_metrics(size, measure, font.line_height, scale);
                tracing::debug!(?cell, "cell metrics");
                self.metrics = Some((cell, font.family.clone(), font.line_height));
                cell
            }
        }
    }

    /// Each shown pane's frame; `None` while it attaches or shows a banner.
    fn pane_frames(&mut self, boxes: &[PaneBox], window: &Window) -> Vec<Option<PaneFrame>> {
        let cell = self.cell_metrics(window);
        let focused = self.focused_pane().filter(|_| window.is_window_active());
        let opacity = self.config.window.opacity as f32;
        let cursor = self.palette.cursor;
        boxes
            .iter()
            .map(|pane_box| {
                let attached = self.panes.get_mut(pane_box.pane)?;
                let State::Live(mirror) = &mut attached.state else {
                    return None;
                };
                let started = Instant::now();
                let settings = draw::Settings {
                    opacity,
                    cursor,
                    focused: focused == Some(pane_box.pane),
                };
                let list = mirror
                    .draw(&settings)
                    .inspect_err(|error| tracing::warn!(%error, "can't draw a pane"))
                    .ok()?;
                Some(attached.painter.prepare(
                    &list,
                    cell,
                    &mut self.glyphs,
                    window.text_system(),
                    started,
                ))
            })
            .collect()
    }

    /// A pane's banner, with Retry, laid over its box.
    fn banner(&mut self, pane_box: &PaneBox, cx: &mut Context<Self>) -> Option<AnyElement> {
        let State::Stopped(banner) = &self.panes.get_mut(pane_box.pane)?.state else {
            return None;
        };
        let pane = pane_box.pane;
        let rect = pane_box.rect;
        let retry = div()
            .id(SharedString::from(format!("retry-{pane}")))
            .px(px(10.0))
            .py(px(3.0))
            .rounded(px(6.0))
            .cursor_pointer()
            .bg(rgba(Color {
                rgb: self.palette.unfocused_border,
                alpha: 1.0,
            }))
            .child("Retry")
            .on_click(cx.listener(move |view, _, _, cx| {
                let ShoalWindow { panes, client, .. } = view;
                panes.retry(pane, client, cx);
                cx.notify();
            }));
        Some(
            div()
                .absolute()
                .left(px(rect.x as f32))
                .top(px(rect.y as f32))
                .w(px(rect.width as f32))
                .h(px(rect.height as f32))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(10.0))
                .p(px(12.0))
                .text_color(rgba(Color {
                    rgb: self.palette.foreground,
                    alpha: 1.0,
                }))
                .child(SharedString::from(banner.text()))
                .child(retry)
                .into_any_element(),
        )
    }
}

impl Render for ShoalWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let width = f32::from(viewport.width).max(0.0) as u32;
        let height = (f32::from(viewport.height).max(0.0) as u32).saturating_sub(TOP_BAR_HEIGHT);
        let Some(chrome) = chrome::chrome(
            &self.workspace,
            self.session,
            &self.palette,
            &self.config.window,
            width,
            height,
        ) else {
            return div().size_full();
        };
        if chrome.title != self.title {
            window.set_window_title(&chrome.title);
            self.title = chrome.title.clone();
        }
        let Chrome {
            bar,
            chip,
            tabs,
            panes,
            ..
        } = chrome;
        let top_bar = div()
            .h(px(TOP_BAR_HEIGHT as f32))
            .w_full()
            .flex_none()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(2.0))
            .px(px(8.0))
            .overflow_hidden()
            .bg(rgba(bar))
            .child(
                div()
                    .flex_none()
                    .mr(px(6.0))
                    .px(px(8.0))
                    .py(px(2.0))
                    .rounded(px(6.0))
                    .bg(rgba(chip.fill))
                    .text_color(rgba(chip.text))
                    .child(SharedString::from(chip.name)),
            )
            .children(tabs.into_iter().map(tab_label));
        let frames = self.pane_frames(&panes, window);
        let banners: Vec<_> = panes
            .iter()
            .filter_map(|pane| self.banner(pane, cx))
            .collect();
        let cell = self.metrics.as_ref().map(|(cell, ..)| *cell);
        let boxes = canvas(
            |_, _, _| (),
            move |bounds, (), window, _| {
                for (pane, frame) in panes.iter().zip(&frames) {
                    let origin =
                        bounds.origin + point(px(pane.rect.x as f32), px(pane.rect.y as f32));
                    let extent = size(px(pane.rect.width as f32), px(pane.rect.height as f32));
                    let pane_bounds = Bounds::new(origin, extent);
                    window.paint_quad(quad(
                        pane_bounds,
                        px(pane.radius),
                        rgba(pane.fill),
                        px(pane.border_width),
                        rgba(pane.border),
                        BorderStyle::Solid,
                    ));
                    let (Some(frame), Some(cell)) = (frame, cell) else {
                        continue;
                    };
                    // A mirror bigger than its box is clipped at the inset.
                    let inset = px(metrics::GRID_INSET);
                    let inner = Bounds::new(
                        origin + point(inset, inset),
                        size(
                            (extent.width - inset * 2.0).max(px(0.0)),
                            (extent.height - inset * 2.0).max(px(0.0)),
                        ),
                    );
                    let (x, y) = metrics::grid_origin(pane.rect, &cell);
                    let grid = bounds.origin + point(px(x), px(y));
                    window.with_content_mask(Some(ContentMask { bounds: inner }), |window| {
                        window.paint_layer(inner, |window| terminal::paint(frame, grid, window));
                    });
                }
            },
        )
        .size_full();
        let pane_area = div()
            .relative()
            .flex_1()
            .w_full()
            .child(boxes)
            .children(banners);
        div()
            .size_full()
            .flex()
            .flex_col()
            .font_family(SharedString::from(self.config.font.family.clone()))
            .text_size(px(TEXT_SIZE))
            .child(top_bar)
            .child(pane_area)
    }
}

/// A tab's glyph and title, at most 200 px wide with an ellipsis, with the
/// active tab's underline and a ringing tab's dot.
fn tab_label(tab: TabLabel) -> Div {
    let mut label = div()
        .relative()
        .h_full()
        .min_w_0()
        .max_w(px(200.0))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .px(px(8.0))
        .text_color(rgba(tab.text))
        .child(
            // Symbols spill past their one-cell advance; give them room.
            div()
                .flex_none()
                .w(px(GLYPH_WIDTH))
                .font_family(fonts::SYMBOLS_FAMILY)
                .child(tab.glyph.text()),
        )
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(SharedString::from(tab.title)),
        );
    if let Some(bell) = tab.bell {
        label = label.child(
            div()
                .flex_none()
                .size(px(6.0))
                .rounded_full()
                .bg(rgba(bell)),
        );
    }
    if let Some(underline) = tab.underline {
        label = label.child(
            div()
                .absolute()
                .bottom_0()
                .left(px(8.0))
                .right(px(8.0))
                .h(px(UNDERLINE_WIDTH))
                .bg(rgba(underline)),
        );
    }
    label
}

pub(crate) fn hsla(color: Color) -> Hsla {
    rgba(color).into()
}

fn rgba(color: Color) -> Rgba {
    let [r, g, b] = color.rgb.0.map(|channel| f32::from(channel) / 255.0);
    Rgba {
        r,
        g,
        b,
        a: color.alpha,
    }
}
