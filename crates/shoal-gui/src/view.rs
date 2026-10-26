//! The window's root view: the top bar and pane boxes from [`chrome`],
//! each pane's mirror drawn inside its box, following the workspace as the
//! daemon changes it.

mod connection;
mod input;
mod layout;
mod mouse;
mod scroll;
mod sessions;
mod switcher;
mod text;
mod topbar;

use std::time::Instant;

use gpui::{
    BorderStyle, Bounds, ContentMask, Context, FocusHandle, Hsla, MouseButton, Rgba, SharedString,
    Subscription, Task, Window, canvas, div, point, prelude::*, px, quad, size,
};
use shoal_client::{
    Client, Event,
    model::{PaneId, SessionId, Tab, TabId, Workspace},
    proto::{Call, Notification},
};
use shoal_config::{Config, Palette};

use crate::{
    chrome::{self, Chrome, Color, PaneBox, TOP_BAR_HEIGHT},
    drag::Drag,
    draw,
    glyphs::{self, GlyphCache},
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

pub struct Initial {
    pub client: Client,
    pub session: SessionId,
    pub workspace: Workspace,
    pub available: bool,
}

pub struct ShoalWindow {
    pub(crate) client: Client,
    session: SessionId,
    workspace: Workspace,
    revisions: session::Revisions,
    config: Config,
    available: bool,
    awaiting_workspace: bool,
    banner_height: u32,
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
    switcher: Option<crate::switcher::Switcher>,
    switcher_focus: FocusHandle,
    switcher_scroll: gpui::ScrollHandle,
    session_busy: bool,
    held_modifiers: gpui::Modifiers,
    hover_point: Option<gpui::Point<gpui::Pixels>>,
    caps_lock: bool,
    pressed: std::collections::BTreeMap<String, PaneId>,
    preedit: Option<String>,
    input_cursor: Option<(u16, u16)>,
    window_active: bool,
    terminal_focus: Option<PaneId>,
    mouse_capture: Option<PaneId>,
    wheel_pending: std::collections::BTreeMap<PaneId, (f32, f32)>,
    pub(crate) selection: Option<crate::selection::Selection>,
    /// The window's keys go here; the rename field's go to its own.
    focus: FocusHandle,
    rename_focus: FocusHandle,
    _activation: Subscription,
}

impl gpui::EventEmitter<shoal_client::Error> for ShoalWindow {}

impl ShoalWindow {
    pub fn new(
        initial: Initial,
        config: Config,
        palette: Palette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let Initial {
            client,
            session,
            workspace,
            available,
        } = initial;
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
            revisions: session::Revisions::new(workspace.rev),
            workspace,
            panes: Panes::new(mirror::Setup::from_config(&config)),
            config,
            available,
            awaiting_workspace: false,
            banner_height: 0,
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
            switcher: None,
            switcher_focus: cx.focus_handle(),
            switcher_scroll: gpui::ScrollHandle::new(),
            session_busy: false,
            held_modifiers: gpui::Modifiers::none(),
            hover_point: None,
            caps_lock: false,
            pressed: Default::default(),
            preedit: None,
            input_cursor: None,
            window_active: window.is_window_active(),
            terminal_focus: None,
            mouse_capture: None,
            wheel_pending: Default::default(),
            selection: None,
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
        if !self.available || self.awaiting_workspace {
            return;
        }
        self.panes
            .sync(&self.workspace, self.session, &self.client, cx);
    }

    /// Apply one connection event. Returns `false` once the session is gone
    /// and the window should close.
    pub fn on_event(&mut self, event: Event, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match event {
            Event::Notification(Notification::WorkspaceChanged(state)) => {
                if !self.revisions.accept(state.workspace.rev) {
                    return true;
                }
                self.workspace = state.workspace;
                self.awaiting_workspace = false;
                if let Some(picker) = &mut self.switcher {
                    picker.refresh(&self.workspace.sessions, self.session);
                }
                if self
                    .selection
                    .as_ref()
                    .is_some_and(|s| !self.shows(s.pane) || self.focused_pane() != Some(s.pane))
                {
                    self.clear_selection();
                }
                self.wheel_pending
                    .retain(|pane, _| self.workspace.panes.contains_key(pane));
                self.sync_terminal_focus();
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
            Event::Notification(Notification::DaemonNotice { .. }) => {}
            Event::Notification(other) => tracing::debug!(?other, "ignored notification"),
            Event::Connected(_) => {
                self.revisions.reconnect();
                self.awaiting_workspace = true;
                // The daemon forgot this window with the old connection. The
                // answer doesn't matter here: a closed session arrives as a
                // workspace change.
                drop(self.client.send(session::show_call(self.session)));
                self.report_focus(window);
                cx.notify();
            }
            Event::Disconnected(error) => {
                self.terminal_focus = None;
                self.pressed.clear();
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
        self.window_active = focused;
        self.sync_terminal_focus();
        tracing::debug!(focused, "window focus");
        drop(self.client.send(Call::WindowSetFocused { focused }));
        if focused {
            self.sizer.reset();
        }
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
        let overlay_colors = [
            hsla(Color {
                rgb: self.palette.selection_background,
                alpha: 0.65,
            }),
            hsla(Color {
                rgb: self.palette.foreground,
                alpha: 1.0,
            }),
        ];
        boxes
            .iter()
            .map(|pane_box| {
                let hover = self
                    .hover_point
                    .filter(|at| {
                        self.held_modifiers.control
                            && chrome::pane_at(boxes, layout::pane_point(*at))
                                == Some(pane_box.pane)
                    })
                    .and_then(|at| self.cell_at(pane_box.pane, at));
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
                let mut list = mirror
                    .draw(&settings)
                    .inspect_err(|error| tracing::warn!(%error, "can't draw a pane"))
                    .ok()?;
                if let Some(at) = hover {
                    list.links = mirror
                        .links(at.row)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|link| (at.row, link.columns))
                        .collect();
                }
                if focused == Some(pane_box.pane) {
                    self.input_cursor = list.cursor.as_ref().map(|c| (c.col, c.row));
                }
                Some(attached.painter.prepare(
                    &list,
                    cell,
                    &mut self.glyphs,
                    window.text_system(),
                    started,
                    overlay_colors,
                ))
            })
            .collect()
    }
}

impl Render for ShoalWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let width = f32::from(viewport.width).max(0.0) as u32;
        let height = (f32::from(viewport.height).max(0.0) as u32).saturating_sub(TOP_BAR_HEIGHT);
        let Some(mut chrome) = chrome::chrome(
            &self.workspace,
            self.session,
            &self.palette,
            &self.config.window,
            width,
            height.saturating_sub(self.banner_height),
        ) else {
            return div().size_full();
        };
        for pane in &mut chrome.panes {
            pane.rect.y += self.banner_height;
        }
        for divider in &mut chrome.dividers {
            divider.area.y += self.banner_height;
            divider.gap.y += self.banner_height;
        }
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
        if self.available && !self.awaiting_workspace {
            self.request_sizes(&panes, &cell, cx);
        }
        let frames = self.pane_frames(&panes, cell, window);
        let banners: Vec<_> = panes
            .iter()
            .filter_map(|pane| self.banner(pane, cx))
            .collect();
        let output_pills = self.output_pills(&panes, cx);
        let handles = self.divider_handles(&dividers);
        let dragging = self.drag.as_ref().map(|drag| drag.axis);
        let drag_listeners = self.drag_listeners(dragging, cx);
        self.shown = layout::Shown {
            panes: panes.clone(),
            dividers,
        };
        let input_view = cx.entity();
        let input_focus = (self.available
            && !self.awaiting_workspace
            && self.rename.is_none()
            && self.switcher.is_none()
            && !self.session_busy)
            .then(|| self.focus.clone());
        let boxes = canvas(
            |_, _, _| (),
            move |bounds, (), window, cx| {
                if let Some(input_focus) = input_focus {
                    window.handle_input(
                        &input_focus,
                        gpui::ElementInputHandler::new(bounds, input_view),
                        cx,
                    );
                }
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
                        window.paint_layer(inner, |window| {
                            terminal::paint(frame, grid, window);
                        });
                    });
                }
            },
        )
        .size_full();
        let preedit = self.preedit.as_ref().and_then(|text| {
            let pane = self
                .shown
                .panes
                .iter()
                .find(|p| Some(p.pane) == self.focused_pane())?;
            let (x, y) = metrics::grid_origin(pane.rect, &cell);
            let (col, row) = self.input_cursor.unwrap_or((0, 0));
            Some(
                div()
                    .absolute()
                    .left(px(x + f32::from(col) * cell.width))
                    .top(px(y + f32::from(row) * cell.height))
                    .bg(rgba(Color {
                        rgb: self.palette.background,
                        alpha: 1.0,
                    }))
                    .text_color(rgba(Color {
                        rgb: self.palette.foreground,
                        alpha: 1.0,
                    }))
                    .text_size(px(cell.font_size))
                    .child(SharedString::from(text.clone())),
            )
        });
        let pane_area = div()
            .relative()
            .flex_1()
            .w_full()
            .on_mouse_down(MouseButton::Left, cx.listener(Self::pointer_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::pointer_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::pointer_down))
            .on_mouse_down(
                MouseButton::Navigate(gpui::NavigationDirection::Back),
                cx.listener(Self::pointer_down),
            )
            .on_mouse_down(
                MouseButton::Navigate(gpui::NavigationDirection::Forward),
                cx.listener(Self::pointer_down),
            )
            .on_scroll_wheel(cx.listener(Self::wheel))
            .child(boxes)
            .children(preedit)
            .children(banners)
            .children(output_pills)
            .children(handles);
        let popover = self.session_popover(window, cx);
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::key_down))
            .on_key_up(cx.listener(Self::key_up))
            .on_modifiers_changed(cx.listener(
                |view, event: &gpui::ModifiersChangedEvent, _, cx| {
                    if view.held_modifiers.control != event.modifiers.control {
                        cx.notify();
                    }
                    view.held_modifiers = event.modifiers;
                    view.caps_lock = event.capslock.on;
                },
            ))
            .font_family(SharedString::from(self.config.font.family.clone()))
            .text_size(px(TEXT_SIZE))
            .child(top_bar)
            .child(pane_area)
            .children(popover)
    }
}

pub(crate) fn hsla(color: Color) -> Hsla {
    rgba(color).into()
}

pub(crate) fn rgba(color: Color) -> Rgba {
    let [r, g, b] = color.rgb.0.map(|channel| f32::from(channel) / 255.0);
    Rgba {
        r,
        g,
        b,
        a: color.alpha,
    }
}
