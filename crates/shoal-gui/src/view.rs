//! The window's root view: the top bar and pane boxes from [`chrome`],
//! each pane's mirror drawn inside its box, following the workspace as the
//! daemon changes it.

mod layout;
mod topbar;

use std::time::Instant;

use gpui::{
    AnyElement, BorderStyle, Bounds, ContentMask, Context, FocusHandle, Hsla, KeyDownEvent,
    MouseButton, MouseDownEvent, Rgba, SharedString, Subscription, Task, Window, canvas, div,
    point, prelude::*, px, quad, size,
};
use shoal_client::{
    Client, Event,
    model::{PaneId, SessionId, Tab, TabId, Workspace},
    proto::{Call, Notification},
};
use shoal_config::{Action, Config, Palette};

use crate::{
    chrome::{self, Chrome, Color, PaneBox, TOP_BAR_HEIGHT},
    drag::Drag,
    draw,
    glyphs::{self, GlyphCache},
    keys,
    metrics::{self, CellMetrics},
    mirror::{self, State},
    panes::Panes,
    rename::Rename,
    session,
    sizing::Sizer,
    terminal::{self, PaneFrame},
};

/// Chrome text size in logical pixels.
const TEXT_SIZE: f32 = 13.0;

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
    /// The boxes and dividers last drawn, for the mouse.
    shown: layout::Shown,
    sizer: Sizer,
    /// Wakes the window when the sizer's wait ends.
    size_timer: Option<(Instant, Task<()>)>,
    drag: Option<Drag>,
    /// Sends the dragged ratio once the throttle allows.
    ratio_timer: Option<Task<()>>,
    rename: Option<Rename>,
    /// The window's keys go here; the rename field's go to its own.
    focus: FocusHandle,
    rename_focus: FocusHandle,
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
        // An inactive window draws its cursors hollow, and the daemon
        // follows the focused window's sizes.
        let activation = cx.observe_window_activation(window, |view, window, cx| {
            view.report_focus(window);
            cx.notify();
        });
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
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
            shown: layout::Shown::default(),
            sizer: Sizer::default(),
            size_timer: None,
            drag: None,
            ratio_timer: None,
            rename: None,
            focus,
            rename_focus: cx.focus_handle(),
            _activation: activation,
        };
        if window.is_window_active() {
            view.report_focus(window);
        }
        view.sync_panes(cx);
        view
    }

    fn sync_panes(&mut self, cx: &mut Context<Self>) {
        self.panes
            .sync(&self.workspace, self.session, &self.client, cx);
    }

    /// Apply one connection event. Returns `false` once the session is gone
    /// and the window should close.
    pub fn on_event(&mut self, event: Event, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match event {
            Event::Notification(Notification::WorkspaceChanged(state)) => {
                // Always the newest: one connection's notifications arrive in
                // order, and a restarted daemon may restore an older `rev`.
                self.workspace = state.workspace;
                self.follow_drag();
                if let Some(rename) = &self.rename
                    && self.tab(rename.tab).is_none()
                {
                    self.finish_rename(None, window, cx);
                }
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
                self.report_focus(window);
                cx.notify();
            }
            Event::Disconnected(error) => {
                // T46 shows the reconnecting overlay.
                tracing::warn!(%error, "lost shoald; reconnecting");
            }
        }
        true
    }

    /// Tell the daemon whether this window has focus. The one that last had
    /// it sizes the panes (§5.6), so on gaining it, ask for every shown
    /// pane's size again.
    fn report_focus(&mut self, window: &Window) {
        let focused = window.is_window_active();
        tracing::debug!(focused, "window focus");
        drop(self.client.send(Call::WindowSetFocused { focused }));
        if focused {
            self.sizer.reset();
        }
    }

    /// The window's own keys. Until T42 it acts only on renaming and
    /// zooming.
    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(action) =
            keys::chord(&event.keystroke).and_then(|chord| self.config.keybinds.get(&chord))
        else {
            return;
        };
        let Some(tab) = self.active_tab() else {
            return;
        };
        match action {
            Action::TabRename => {
                let tab = tab.id;
                self.start_rename(tab, window, cx);
            }
            Action::PaneZoom => {
                let call = Call::PaneZoom {
                    pane: tab.focused,
                    zoomed: tab.zoomed.is_none(),
                };
                drop(self.client.send(call));
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    fn shows_session(&self) -> bool {
        self.workspace.sessions.iter().any(|s| s.id == self.session)
    }

    fn tabs(&self) -> &[Tab] {
        self.workspace
            .sessions
            .iter()
            .find(|s| s.id == self.session)
            .map_or(&[], |session| &session.tabs)
    }

    fn tab(&self, id: TabId) -> Option<&Tab> {
        self.tabs().iter().find(|tab| tab.id == id)
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
    fn pane_frames(
        &mut self,
        boxes: &[PaneBox],
        cell: CellMetrics,
        window: &Window,
    ) -> Vec<Option<PaneFrame>> {
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
            dividers,
            ..
        } = chrome;
        let top_bar = self.top_bar(bar, chip, tabs, cx);
        let cell = self.cell_metrics(window);
        self.request_sizes(&panes, &cell, cx);
        let frames = self.pane_frames(&panes, cell, window);
        let banners: Vec<_> = panes
            .iter()
            .filter_map(|pane| self.banner(pane, cx))
            .collect();
        let handles = self.divider_handles(&dividers);
        let dragging = self.drag.as_ref().map(|drag| drag.axis);
        let drag_listeners = self.drag_listeners(dragging, cx);
        self.shown = layout::Shown {
            panes: panes.clone(),
            dividers,
        };
        let boxes = canvas(
            |_, _, _| (),
            move |bounds, (), window, cx| {
                drag_listeners(window, cx);
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
                    let Some(frame) = frame else {
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
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, event: &MouseDownEvent, _, cx| view.press(event.position, cx)),
            )
            .child(boxes)
            .children(banners)
            .children(handles);
        div()
            .size_full()
            .flex()
            .flex_col()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .font_family(SharedString::from(self.config.font.family.clone()))
            .text_size(px(TEXT_SIZE))
            .child(top_bar)
            .child(pane_area)
    }
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
