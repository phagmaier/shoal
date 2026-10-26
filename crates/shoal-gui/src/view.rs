//! The window's root view: the top bar and pane boxes from [`chrome`],
//! following the workspace as the daemon changes it.

use gpui::{
    BorderStyle, Bounds, Context, Div, Rgba, SharedString, Window, canvas, div, point, prelude::*,
    px, quad, size,
};
use shoal_client::{
    Client, Event,
    model::{SessionId, Workspace},
    proto::Notification,
};
use shoal_config::{Config, Palette};

use crate::{
    chrome::{self, Chrome, Color, TOP_BAR_HEIGHT, TabLabel, UNDERLINE_WIDTH},
    fonts, session,
};

/// Chrome text size in logical pixels.
const TEXT_SIZE: f32 = 13.0;
const GLYPH_WIDTH: f32 = 16.0;

pub struct ShoalWindow {
    client: Client,
    session: SessionId,
    workspace: Workspace,
    config: Config,
    palette: Palette,
    title: String,
}

impl ShoalWindow {
    pub fn new(
        client: Client,
        session: SessionId,
        workspace: Workspace,
        config: Config,
        palette: Palette,
    ) -> Self {
        Self {
            client,
            session,
            workspace,
            config,
            palette,
            title: String::new(),
        }
    }

    /// Apply one connection event. Returns `false` once the session is gone
    /// and the window should close.
    pub fn on_event(&mut self, event: Event, cx: &mut Context<Self>) -> bool {
        match event {
            Event::Notification(Notification::WorkspaceChanged(state)) => {
                // Always the newest: one connection's notifications arrive in
                // order, and a restarted daemon may restore an older `rev`.
                self.workspace = state.workspace;
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
}

impl Render for ShoalWindow {
    fn render(&mut self, window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
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
        let pane_area = canvas(
            |_, _, _| (),
            move |bounds, (), window, _| {
                for pane in &panes {
                    let origin =
                        bounds.origin + point(px(pane.rect.x as f32), px(pane.rect.y as f32));
                    let extent = size(px(pane.rect.width as f32), px(pane.rect.height as f32));
                    window.paint_quad(quad(
                        Bounds::new(origin, extent),
                        px(pane.radius),
                        rgba(pane.fill),
                        px(pane.border_width),
                        rgba(pane.border),
                        BorderStyle::Solid,
                    ));
                }
            },
        )
        .flex_1()
        .w_full();
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

fn rgba(color: Color) -> Rgba {
    let [r, g, b] = color.rgb.0.map(|channel| f32::from(channel) / 255.0);
    Rgba {
        r,
        g,
        b,
        a: color.alpha,
    }
}
