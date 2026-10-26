//! The session-chip popover, text input and keyboard controls (§7.6).
use super::{ShoalWindow, rgba};
use crate::chrome::{Color, TOP_BAR_HEIGHT};
use crate::{
    field::{Outcome, TextField},
    keys,
    switcher::{Mode, Target},
};
use gpui::{AnyElement, Context, KeyDownEvent, SharedString, Window, div, prelude::*, px};
use shoal_config::Action;

impl ShoalWindow {
    pub(super) fn session_popover(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let picker = self.switcher.as_ref()?;
        let color = |rgb| rgba(Color { rgb, alpha: 1.0 });
        let rows = picker.rows(&self.workspace.sessions, self.session);
        let mut popover = div()
            .id("session-switcher")
            .absolute()
            .left(px(8.0))
            .top(px(TOP_BAR_HEIGHT as f32))
            .w(px(360.0))
            .max_w((window.viewport_size().width - px(16.0)).max(px(0.0)))
            .max_h((window.viewport_size().height - px(TOP_BAR_HEIGHT as f32 + 8.0)).max(px(0.0)))
            .flex()
            .flex_col()
            .gap(px(6.0))
            .p(px(8.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(color(self.palette.unfocused_border))
            .shadow_lg()
            .bg(color(self.palette.top_bar))
            .text_color(color(self.palette.foreground))
            .track_focus(&self.switcher_focus)
            .occlude()
            .on_key_down(cx.listener(Self::switcher_key))
            .on_mouse_down_out(cx.listener(|view, _, window, cx| {
                view.dismiss_switcher(window, cx);
                cx.stop_propagation();
            }));
        match &picker.mode {
            Mode::Browse => {
                if !picker.busy {
                    popover = popover.child(self.text_field(
                        &picker.query,
                        &self.switcher_focus,
                        "session-filter",
                        "Filter sessions",
                        cx,
                    ));
                }
                popover = popover.child(
                    div()
                        .id("session-rows")
                        .min_h_0()
                        .flex_shrink(1.0)
                        .overflow_y_scroll()
                        .track_scroll(&self.switcher_scroll)
                        .children(rows.into_iter().map(|row| {
                            let target = row.target;
                            let selected = picker.selected == target;
                            let mut label = div()
                                .id(SharedString::from(format!("session-{:?}", target)))
                                .flex_none()
                                .flex()
                                .items_center()
                                .gap(px(8.0))
                                .px(px(8.0))
                                .py(px(5.0))
                                .rounded(px(4.0))
                                .cursor_pointer()
                                .when(selected, |div| {
                                    div.bg(rgba(Color {
                                        rgb: self.palette.accent,
                                        alpha: 0.22,
                                    }))
                                })
                                .hover(|style| style.bg(color(self.palette.unfocused_border)))
                                .child(
                                    div()
                                        .min_w_0()
                                        .flex_1()
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .child(SharedString::from(row.name)),
                                );
                            if let Target::Session(session) = target {
                                if session == self.session {
                                    label = label.child(
                                        div()
                                            .text_color(color(self.palette.accent))
                                            .child("Current"),
                                    );
                                }
                                label = label.child(
                                    div().text_color(color(self.palette.inactive_text)).child(
                                        SharedString::from(format!(
                                            "{} {}",
                                            row.tabs,
                                            if row.tabs == 1 { "tab" } else { "tabs" }
                                        )),
                                    ),
                                );
                                if row.on_screen {
                                    label = label.child(
                                        div()
                                            .text_color(color(self.palette.inactive_text))
                                            .child("On screen"),
                                    );
                                }
                            }
                            label.on_click(cx.listener(move |view, _, window, cx| {
                                view.choose_session(target, window, cx)
                            }))
                        })),
                );
                popover = popover.child(
                    div()
                        .text_size(px(11.0))
                        .text_color(color(self.palette.inactive_text))
                        .child("↑ ↓ Select · Enter Switch · F2 Rename")
                        .child(div().child("Ctrl+Backspace Close · Esc Dismiss")),
                );
            }
            Mode::Rename { field, .. } => {
                popover = popover.child("Rename session");
                if !picker.busy {
                    popover = popover.child(self.text_field(
                        field,
                        &self.switcher_focus,
                        "session-name",
                        "Session name",
                        cx,
                    ));
                }
                popover = popover.child(
                    div()
                        .text_size(px(11.0))
                        .text_color(color(self.palette.inactive_text))
                        .child("Enter Save · Esc Cancel"),
                );
            }
            Mode::Confirm { name, .. } => {
                popover = popover
                    .child(SharedString::from(format!("Close session {name}?")))
                    .child(
                        div()
                            .text_color(color(self.palette.inactive_text))
                            .child("Enter / Esc"),
                    );
            }
        }
        if picker.busy {
            popover = popover.child("Working…");
        }
        if let Some(error) = &picker.error {
            popover = popover.child(
                div()
                    .text_color(color(self.palette.accent))
                    .child(SharedString::from(error.clone())),
            );
        }
        Some(popover.into_any_element())
    }

    fn choose_session(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        if self.session_busy {
            return;
        }
        match target {
            Target::Session(session) => self.switch_session(session, window, cx),
            Target::New => self.create_session(window, cx),
        }
    }

    fn switcher_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picker) = &mut self.switcher else {
            return;
        };
        let key = &event.keystroke;
        if key.key == "escape" {
            if matches!(picker.mode, Mode::Browse) || picker.busy {
                self.dismiss_switcher(window, cx);
            } else {
                picker.mode = Mode::Browse;
                picker.error = None;
                cx.notify();
            }
            cx.stop_propagation();
            return;
        }
        if picker.busy {
            cx.stop_propagation();
            return;
        }
        // Key text is delivered exactly once through EntityInputHandler.
        if key.key_char.is_some()
            && !key.modifiers.control
            && !key.modifiers.alt
            && !key.modifiers.platform
        {
            return;
        }
        let action = keys::resolve(key, self.held_modifiers, &self.config.keybinds);
        match action {
            Some(Action::SessionSwitcher) => self.dismiss_switcher(window, cx),
            Some(Action::ClipboardPaste) => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text())
                    && let Some(field) = picker.field_mut()
                {
                    field.replace(None, &text, None, false);
                    self.filter_changed();
                    cx.notify();
                }
            }
            Some(Action::ClipboardCopy) => {
                if let Some(field) = picker.field().filter(|f| f.all_selected()) {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(field.text().into()));
                }
            }
            _ => match &mut picker.mode {
                Mode::Confirm { session, .. } => {
                    if key.key == "enter" && !event.is_held {
                        let session = *session;
                        self.close_session(session, window, cx);
                    }
                }
                Mode::Rename { session, field } => {
                    if key.modifiers.control && key.key == "a" {
                        *field = TextField::new(field.text());
                        cx.notify();
                    } else {
                        match field.key(&key.key, None) {
                            Outcome::Commit(Some(Some(name))) => {
                                let session = *session;
                                self.rename_session(session, name, window, cx);
                            }
                            Outcome::Commit(Some(None)) => {
                                picker.error = Some("Enter a session name".into());
                                cx.notify();
                            }
                            Outcome::Commit(None) => {
                                picker.mode = Mode::Browse;
                                picker.error = None;
                                cx.notify();
                            }
                            _ => cx.notify(),
                        }
                    }
                }
                Mode::Browse => match key.key.as_str() {
                    "up" | "down" => {
                        let index =
                            picker.step(&self.workspace.sessions, self.session, key.key == "down");
                        self.switcher_scroll.scroll_to_item(index);
                        cx.notify();
                    }
                    "enter" => {
                        if !event.is_held {
                            let target = picker.selected;
                            self.choose_session(target, window, cx);
                        }
                    }
                    "f2" | "backspace" if key.key == "f2" || key.modifiers.control => {
                        if let Target::Session(id) = picker.selected
                            && let Some(session) =
                                self.workspace.sessions.iter().find(|s| s.id == id)
                        {
                            picker.mode = if key.key == "f2" {
                                Mode::Rename {
                                    session: id,
                                    field: TextField::new(&session.name),
                                }
                            } else {
                                Mode::Confirm {
                                    session: id,
                                    name: session.name.clone(),
                                }
                            };
                            picker.error = None;
                            cx.notify();
                        }
                    }
                    "a" if key.modifiers.control => {
                        picker.query = TextField::new(picker.query.text());
                        cx.notify();
                    }
                    _ => {
                        picker.query.key(&key.key, None);
                        self.filter_changed();
                        cx.notify();
                    }
                },
            },
        }
        cx.stop_propagation();
    }
}
