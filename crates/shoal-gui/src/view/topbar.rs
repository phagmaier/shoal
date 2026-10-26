//! The top bar (M1 spec §7.2): the session chip, tabs that select on
//! click, close on middle-click and rename inline on double-click, the `+`
//! button and the split buttons, whose tooltips show their shortcuts as
//! key caps.

use gpui::{
    AnyElement, ClickEvent, Context, Div, KeyDownEvent, MouseButton, SharedString, Window, div,
    prelude::*, px,
};
use shoal_client::{
    model::{SplitDirection, TabId},
    proto::Call,
};
use shoal_config::Action;

use super::{ShoalWindow, rgba};
use crate::{
    chrome::{self, Chip, Color, TOP_BAR_HEIGHT, TabLabel, UNDERLINE_WIDTH},
    fonts, keys,
    rename::{Outcome, Rename},
};

const GLYPH_WIDTH: f32 = 16.0;
/// Codicons `add`, `split-horizontal` (side by side) and `split-vertical`
/// (stacked), from the bundled symbols font.
const NEW_TAB_GLYPH: &str = "\u{ea60}";
const SPLIT_RIGHT_GLYPH: &str = "\u{eb56}";
const SPLIT_DOWN_GLYPH: &str = "\u{eb57}";

impl ShoalWindow {
    pub(super) fn top_bar(
        &self,
        bar: Color,
        chip: Chip,
        tabs: Vec<TabLabel>,
        cx: &mut Context<Self>,
    ) -> Div {
        let labels: Vec<_> = tabs
            .into_iter()
            .map(|tab| self.tab_label(tab, cx))
            .collect();
        let session = self.session;
        let tooltip = self.action_tooltip("Session switcher", Action::SessionSwitcher);
        div()
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
                    .id("session-chip")
                    .tooltip(move |_, cx| cx.new(|_| tooltip.clone()).into())
                    .cursor_pointer()
                    .on_click(cx.listener(|view, _, window, cx| view.toggle_switcher(window, cx)))
                    .flex_none()
                    .mr(px(6.0))
                    .px(px(8.0))
                    .py(px(2.0))
                    .rounded(px(6.0))
                    .bg(rgba(chip.fill))
                    .text_color(rgba(chip.text))
                    .child(SharedString::from(chip.name)),
            )
            .child(
                div()
                    .h_full()
                    .min_w_0()
                    .flex()
                    .flex_row()
                    .overflow_hidden()
                    .children(labels),
            )
            .child(self.button(
                "new-tab",
                NEW_TAB_GLYPH,
                "New tab",
                Action::TabNew,
                move |_| {
                    Some(Call::TabCreate {
                        session,
                        cwd: None,
                        index: None,
                        argv: None,
                    })
                },
                cx,
            ))
            .child(div().flex_1())
            .child(self.button(
                "split-right",
                SPLIT_RIGHT_GLYPH,
                "Split right",
                Action::PaneSplitRight,
                |view| view.split_call(SplitDirection::Right),
                cx,
            ))
            .child(self.button(
                "split-down",
                SPLIT_DOWN_GLYPH,
                "Split down",
                Action::PaneSplitDown,
                |view| view.split_call(SplitDirection::Down),
                cx,
            ))
    }

    fn split_call(&self, direction: SplitDirection) -> Option<Call> {
        Some(Call::PaneSplit {
            pane: self.focused_pane()?,
            direction,
            cwd: None,
        })
    }

    /// A tab's glyph and title, at most 200 px wide with an ellipsis, with
    /// the active tab's underline and a ringing tab's dot.
    fn tab_label(&self, tab: TabLabel, cx: &mut Context<Self>) -> AnyElement {
        let id = tab.id;
        let active = tab.underline.is_some();
        let title = match self.rename.as_ref().filter(|rename| rename.tab == id) {
            Some(rename) => self.rename_field(rename, cx),
            None => div()
                .min_w_0()
                .overflow_hidden()
                .whitespace_nowrap()
                .text_ellipsis()
                .child(SharedString::from(tab.title))
                .into_any_element(),
        };
        let mut label = div()
            .id(SharedString::from(format!("tab-{id}")))
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
            .on_click(cx.listener(move |view, event: &ClickEvent, window, cx| {
                if view.close_confirmation.is_some() {
                    return;
                }
                if event.click_count() >= 2 {
                    view.start_rename(id, window, cx);
                } else if !active {
                    drop(view.client.send(Call::TabActivate { tab: id }));
                }
            }))
            .on_mouse_up(
                MouseButton::Middle,
                cx.listener(move |view, _, window, cx| {
                    view.request_close(crate::closing::Target::Tab(id), window, cx);
                }),
            )
            .child(
                // Symbols spill past their one-cell advance; give them room.
                div()
                    .flex_none()
                    .w(px(GLYPH_WIDTH))
                    .font_family(fonts::SYMBOLS_FAMILY)
                    .child(tab.glyph.text()),
            )
            .child(title);
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
        label.into_any_element()
    }

    /// A top-bar button: a glyph, a tooltip with the action's key caps, and
    /// the call a click sends.
    fn button(
        &self,
        id: &'static str,
        glyph: &'static str,
        label: &'static str,
        action: Action,
        call: impl Fn(&ShoalWindow) -> Option<Call> + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let palette = &self.palette;
        let tooltip = self.action_tooltip(label, action);
        let hover_fill = Color {
            rgb: palette.unfocused_border,
            alpha: 0.8,
        };
        let hover_text = opaque(palette.foreground);
        div()
            .id(id)
            .flex_none()
            .w(px(26.0))
            .h(px(24.0))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.0))
            .cursor_pointer()
            .font_family(fonts::SYMBOLS_FAMILY)
            .text_size(px(15.0))
            .text_color(rgba(opaque(palette.inactive_text)))
            .hover(move |style| style.bg(rgba(hover_fill)).text_color(rgba(hover_text)))
            .child(glyph)
            .tooltip(move |_, cx| cx.new(|_| tooltip.clone()).into())
            .on_click(cx.listener(move |view, _, _, _| {
                if view.close_confirmation.is_some() {
                    return;
                }
                if let Some(call) = call(view) {
                    drop(view.client.send(call));
                }
            }))
    }

    fn action_tooltip(&self, label: &'static str, action: Action) -> Tooltip {
        let palette = &self.palette;
        Tooltip {
            label,
            caps: keys::shortcut(&self.config.keybinds, action)
                .map(|chord| keys::caps(&chord))
                .unwrap_or_default(),
            family: SharedString::from(self.config.font.family.clone()),
            fill: opaque(palette.top_bar),
            border: opaque(palette.unfocused_border),
            text: opaque(palette.inactive_text),
            cap_fill: opaque(palette.unfocused_border),
            cap_text: opaque(palette.foreground),
        }
    }

    /// The inline rename field in place of a tab's title: the text, its
    /// caret, and the whole label selected until the first key.
    fn rename_field(&self, rename: &Rename, cx: &mut Context<Self>) -> AnyElement {
        self.text_field(&rename.field, &self.rename_focus, "rename", "", cx)
            .on_key_down(cx.listener(Self::rename_key))
            .on_mouse_down_out(cx.listener(|view, _, window, cx| {
                let name = view.rename.as_ref().and_then(Rename::name);
                view.finish_rename(name, window, cx);
            }))
            .into_any_element()
    }

    pub(super) fn start_rename(&mut self, tab: TabId, window: &mut Window, cx: &mut Context<Self>) {
        if self.close_confirmation.is_some() {
            return;
        }
        if self.rename.as_ref().is_some_and(|rename| rename.tab == tab) {
            return;
        }
        let Some(found) = self.tab(tab) else {
            return;
        };
        let label = chrome::tab_title(&self.workspace, found);
        self.rename = Some(Rename::new(tab, &label));
        self.preedit = None;
        self.sync_terminal_focus();
        window.focus(&self.rename_focus, cx);
        cx.notify();
    }

    fn rename_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(rename) = &mut self.rename else {
            return;
        };
        let keystroke = &event.keystroke;
        match keys::resolve(keystroke, self.held_modifiers, &self.config.keybinds) {
            Some(Action::ClipboardPaste) => {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    rename.field.replace(None, &text, None, false);
                    cx.notify();
                }
                cx.stop_propagation();
                return;
            }
            Some(Action::ClipboardCopy) => {
                if rename.field.all_selected() {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                        rename.field.text().into(),
                    ));
                }
                cx.stop_propagation();
                return;
            }
            _ => {}
        }
        let modifiers = keystroke.modifiers;
        // Text, including composition, arrives through EntityInputHandler.
        if keystroke.key_char.is_some()
            && !modifiers.control
            && !modifiers.alt
            && !modifiers.platform
        {
            return;
        }
        match rename.field.key(&keystroke.key, None) {
            Outcome::Editing => cx.notify(),
            Outcome::Commit(name) => self.finish_rename(name, window, cx),
            Outcome::Cancel => self.finish_rename(None, window, cx),
        }
        cx.stop_propagation();
    }

    /// End renaming, sending `name` if there is one, and give the keys back
    /// to the window.
    pub(super) fn finish_rename(
        &mut self,
        name: Option<Option<String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(rename) = self.rename.take() else {
            return;
        };
        if let Some(name) = name {
            drop(self.client.send(Call::TabRename {
                tab: rename.tab,
                name,
            }));
        }
        window.focus(&self.focus, cx);
        self.sync_terminal_focus();
        cx.notify();
    }
}

/// A button's tooltip: what it does and its shortcut's key caps.
#[derive(Clone)]
struct Tooltip {
    label: &'static str,
    caps: Vec<String>,
    family: SharedString,
    fill: Color,
    border: Color,
    text: Color,
    cap_fill: Color,
    cap_text: Color,
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let cap_fill = self.cap_fill;
        let cap_text = self.cap_text;
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(4.0))
            .px(px(8.0))
            .py(px(4.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(rgba(self.border))
            .bg(rgba(self.fill))
            .font_family(self.family.clone())
            .text_size(px(12.0))
            .text_color(rgba(self.text))
            .child(div().mr(px(4.0)).child(self.label))
            .children(self.caps.iter().map(|cap| {
                div()
                    .px(px(5.0))
                    .rounded(px(4.0))
                    .bg(rgba(cap_fill))
                    .text_color(rgba(cap_text))
                    .child(SharedString::from(cap.clone()))
            }))
    }
}

fn opaque(rgb: shoal_config::schema::Rgb) -> Color {
    Color { rgb, alpha: 1.0 }
}
