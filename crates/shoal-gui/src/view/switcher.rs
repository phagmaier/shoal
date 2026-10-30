//! The centered session switcher, text input and keyboard controls (§7.6).
use super::{ShoalWindow, rgba};
use crate::chrome::{Color, Glyph};
use crate::{
    controls::{self, Looks},
    field::{Outcome, TextField},
    fonts, keys,
    switcher::{Edit, Mode, Row, Target},
};
use gpui::{AnyElement, Context, KeyDownEvent, SharedString, Window, div, prelude::*, px};
use shoal_client::model::SessionId;
use shoal_config::Action;

/// The search row leads the card, a size up from the rest of the chrome.
const SEARCH_TEXT_SIZE: f32 = 16.0;

impl ShoalWindow {
    /// The centered switcher card: a filter, the sessions,
    /// and the keys that work here.
    pub(super) fn session_popover(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let picker = self.switcher.as_ref()?;
        let dim = Color {
            rgb: self.palette.foreground,
            alpha: 0.9,
        };
        let looks = Looks {
            cap_fill: Color {
                rgb: self.palette.foreground,
                alpha: 0.05,
            },
            outline: Color {
                rgb: self.palette.foreground,
                alpha: 0.10,
            },
            cap_text: dim,
            hint: dim,
            ..Looks::new(&self.palette)
        };
        let rule = rgba(Color {
            rgb: self.palette.foreground,
            alpha: 0.08,
        });
        let section = |child: gpui::Div| child.px(px(16.0)).py(px(10.0));
        let mut card = div()
            .id("session-switcher")
            .w(px(640.0))
            .max_w((window.viewport_size().width - px(32.0)).max(px(0.0)))
            .max_h((window.viewport_size().height - px(32.0)).max(px(0.0)))
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(16.0))
            .border_1()
            .border_color(rule)
            .shadow_lg()
            .bg(rgba(Color::opaque(self.palette.unfocused_border)))
            .text_color(rgba(looks.text))
            .track_focus(&self.switcher_focus)
            .occlude()
            .on_key_down(cx.listener(Self::switcher_key))
            .on_mouse_down_out(cx.listener(|view, _, window, cx| {
                view.dismiss_switcher(window, cx);
                cx.stop_propagation();
            }));
        let footer = match &picker.mode {
            Mode::Browse => {
                if !picker.busy {
                    card = card.child(
                        section(div())
                            .flex()
                            .items_center()
                            .gap(px(12.0))
                            .min_h(px(54.0))
                            .flex_none()
                            .border_b_1()
                            .border_color(rule)
                            .child(
                                controls::glyph(controls::SEARCH_GLYPH, dim)
                                    .w(px(20.0))
                                    .text_size(px(SEARCH_TEXT_SIZE)),
                            )
                            .child(
                                self.text_field(
                                    &picker.query,
                                    &self.switcher_focus,
                                    "session-filter",
                                    "Find a session…",
                                    SEARCH_TEXT_SIZE,
                                    cx,
                                )
                                .flex_1()
                                .min_w_0()
                                .bg(gpui::transparent_black()),
                            )
                            .child(controls::cap("Esc", &looks).text_size(px(11.0))),
                    );
                }
                let rows = picker.rows(&self.workspace.sessions, self.session);
                card = card.child(
                    div()
                        .id("session-rows")
                        .min_h_0()
                        .flex_shrink(1.0)
                        .overflow_y_scroll()
                        .track_scroll(&self.switcher_scroll)
                        .p(px(8.0))
                        .children(rows.into_iter().map(|row| {
                            let target = row.target;
                            self.session_row(row, picker.selected == target, &looks, cx)
                                .on_click(cx.listener(move |view, _, window, cx| {
                                    view.choose_session(target, window, cx)
                                }))
                        })),
                );
                controls::hints([
                    controls::hint(&["↑", "↓"], "Select", &looks),
                    controls::hint(&["Enter"], "Switch", &looks),
                    controls::hint(&["F2"], "Rename", &looks),
                    controls::hint(&["Ctrl", "Backspace"], "Close", &looks),
                    controls::hint(&["Esc"], "Dismiss", &looks),
                ])
            }
            Mode::Rename { field, .. } => {
                let mut body = section(div())
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(div().text_color(rgba(dim)).child("Rename session"));
                if !picker.busy {
                    body = body.child(self.text_field(
                        field,
                        &self.switcher_focus,
                        "session-name",
                        "Session name",
                        super::TEXT_SIZE,
                        cx,
                    ));
                }
                card = card.child(body);
                controls::hints([
                    controls::hint(&["Enter"], "Save", &looks),
                    controls::hint(&["Esc"], "Cancel", &looks),
                ])
            }
            Mode::Confirm { session, name } => {
                let text = crate::closing::Target::Session(*session)
                    .prompt(&self.workspace, self.config.behavior.confirm_close_running)
                    .unwrap_or_else(|| format!("Close session {name}?"));
                card = card.child(
                    section(div())
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(controls::glyph(controls::WARNING_GLYPH, looks.accent))
                        .child(div().min_w_0().flex_1().child(SharedString::from(text))),
                );
                controls::hints([
                    controls::hint(&["Enter"], "Close", &looks),
                    controls::hint(&["Esc"], "Cancel", &looks),
                ])
            }
        };
        if picker.busy {
            card = card.child(section(div()).text_color(rgba(dim)).child("Working…"));
        }
        if let Some(error) = &picker.error {
            card = card.child(
                section(div())
                    .text_color(rgba(Color::opaque(self.palette.ansi[1])))
                    .child(SharedString::from(error.clone())),
            );
        }
        card = card.child(
            section(footer)
                .text_size(px(12.0))
                .flex_none()
                .border_t_1()
                .border_color(rule),
        );
        Some(
            div()
                .absolute()
                .left_0()
                .top_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgba(Color {
                    rgb: self.palette.top_bar,
                    alpha: 0.35,
                }))
                .occlude()
                .child(card)
                .into_any_element(),
        )
    }

    /// One switcher row: the session's glyph, name and details, or New
    /// session. The selected row is tinted, with an accent edge; it and a
    /// row under the pointer show rename and close buttons.
    fn session_row(
        &self,
        row: Row,
        selected: bool,
        looks: &Looks,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let dim = rgba(looks.hint);
        // A step below the selected row's tint, visible on the card.
        let hover = rgba(Color {
            rgb: self.palette.foreground,
            alpha: 0.035,
        });
        let group = SharedString::from(format!("session-row-{:?}", row.target));
        let (glyph, current, details) = match row.target {
            Target::Session(session) => (
                Glyph::Session.text(),
                session == self.session,
                Some((session, row.tabs, row.on_screen)),
            ),
            Target::New => (controls::ADD_GLYPH, false, None),
        };
        let mut label = div()
            .id(SharedString::from(format!("session-{:?}", row.target)))
            .group(group.clone())
            .flex_none()
            .flex()
            .items_center()
            .gap(px(10.0))
            .px(px(10.0))
            .min_h(px(42.0))
            .py(px(7.0))
            .relative()
            .rounded(px(9.0))
            .cursor_pointer()
            .when(selected, |row| {
                row.bg(rgba(Color {
                    rgb: self.palette.foreground,
                    alpha: 0.06,
                }))
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .top(px(10.0))
                        .bottom(px(10.0))
                        .w(px(2.0))
                        .rounded_full()
                        .bg(rgba(looks.accent)),
                )
            })
            .when(!selected, |row| row.hover(move |style| style.bg(hover)))
            .child(
                div()
                    .flex_none()
                    .size(px(24.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.0))
                    .when(selected, |icon| {
                        icon.bg(rgba(Color {
                            rgb: self.palette.top_bar,
                            alpha: 0.6,
                        }))
                    })
                    .child(if current {
                        div().child("✓")
                    } else {
                        controls::glyph(glyph, if selected { looks.text } else { looks.hint })
                    }),
            )
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(SharedString::from(row.name)),
            );
        let Some((session, tabs, on_screen)) = details else {
            return label;
        };
        let mut details = format!("{tabs} {}", if tabs == 1 { "tab" } else { "tabs" });
        if current {
            details.push_str(" · Current");
        }
        if on_screen {
            details.push_str(" · On screen");
        }
        label = label
            .child(
                div()
                    .min_w_0()
                    .max_w(gpui::relative(0.5))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .text_size(px(12.0))
                    .text_color(dim)
                    .child(SharedString::from(details)),
            )
            .child(div().flex_1())
            .child(self.row_button(session, Edit::Rename, selected, &group, looks, cx))
            .child(self.row_button(session, Edit::Close, selected, &group, looks, cx));
        label
    }

    /// A row's rename or close button: the same edit as F2 or
    /// Ctrl+Backspace, shown on the selected row and the one under the pointer.
    fn row_button(
        &self,
        session: SessionId,
        edit: Edit,
        selected: bool,
        group: &SharedString,
        looks: &Looks,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let hover_fill = rgba(Color {
            rgb: self.palette.foreground,
            alpha: 0.10,
        });
        let hover_text = rgba(looks.text);
        div()
            .id(SharedString::from(format!("{edit:?}-{session}")))
            .flex_none()
            .size(px(24.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .font_family(fonts::SYMBOLS_FAMILY)
            .text_size(px(14.0))
            .text_color(rgba(looks.hint))
            .hover(move |style| style.bg(hover_fill).text_color(hover_text))
            .when(!selected, |button| {
                button
                    .invisible()
                    .group_hover(group.clone(), |style| style.visible())
            })
            .child(match edit {
                Edit::Rename => controls::EDIT_GLYPH,
                Edit::Close => controls::CLOSE_GLYPH,
            })
            .on_click(cx.listener(move |view, _, _, cx| {
                // The row under the button would switch to the session.
                cx.stop_propagation();
                if view.session_busy {
                    return;
                }
                if let Some(picker) = view.switcher.as_mut().filter(|picker| !picker.busy)
                    && picker.edit(&view.workspace.sessions, session, edit)
                {
                    cx.notify();
                }
            }))
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
                        let edit = if key.key == "f2" {
                            Edit::Rename
                        } else {
                            Edit::Close
                        };
                        if let Target::Session(id) = picker.selected
                            && picker.edit(&self.workspace.sessions, id, edit)
                        {
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
