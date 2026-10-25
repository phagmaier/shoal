//! Encoding mouse events for programs that turned on mouse reporting
//! (M1 spec §7.4).

use libghostty_vt::mouse::{self, Encoder, EncoderSize, Event, Position};

use crate::{Mods, TermError, Terminal};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    WheelLeft,
    WheelRight,
    Back,
    Forward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    Press,
    Release,
    Motion,
}

/// One mouse event for [`Terminal::encode_mouse`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MouseEvent {
    pub action: MouseAction,
    /// The button pressed or released, or held during motion. `None` for
    /// motion with no button held.
    pub button: Option<MouseButton>,
    pub mods: Mods,
    /// Position in pixels from the grid's top-left corner. It may lie
    /// outside the grid while a button is held.
    pub x: f32,
    pub y: f32,
    /// Whether any button is held, counting this event's own press.
    pub buttons_held: bool,
}

/// The size of one cell in pixels, as drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellSize {
    pub width: u32,
    pub height: u32,
}

pub(crate) struct MouseEncoder {
    encoder: Encoder<'static>,
    event: Event<'static>,
    /// The last report sent. libghostty's own motion dedupe forgets its cell
    /// whenever options are set, and they are set before every event so the
    /// program's latest modes always apply.
    last: Vec<u8>,
}

impl std::fmt::Debug for MouseEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MouseEncoder")
    }
}

impl Terminal {
    /// Whether the program asked for mouse events (modes 9, 1000, 1002 or
    /// 1003). When it didn't, the window handles the mouse itself.
    pub fn mouse_reporting(&self) -> Result<bool, TermError> {
        Ok(self.inner.is_mouse_tracking()?)
    }

    /// Append the bytes for a mouse event under the program's tracking mode
    /// and format (X10, UTF-8, SGR, urxvt or SGR pixels). Events the mode
    /// doesn't report produce nothing, and motion that stays in one cell is
    /// reported once.
    pub fn encode_mouse(
        &mut self,
        event: &MouseEvent,
        cell: CellSize,
        out: &mut Vec<u8>,
    ) -> Result<(), TermError> {
        if cell.width == 0 || cell.height == 0 {
            return Err(TermError(libghostty_vt::Error::InvalidValue));
        }
        let (cols, rows) = (u32::from(self.cols()?), u32::from(self.rows()?));
        if self.view.mouse.is_none() {
            self.view.mouse = Some(MouseEncoder {
                encoder: Encoder::new()?,
                event: Event::new()?,
                last: Vec::new(),
            });
        }
        let state = self
            .view
            .mouse
            .as_mut()
            .expect("invariant: the mouse encoder was just made");
        state
            .event
            .set_action(match event.action {
                MouseAction::Press => mouse::Action::Press,
                MouseAction::Release => mouse::Action::Release,
                MouseAction::Motion => mouse::Action::Motion,
            })
            .set_button(event.button.map(|button| match button {
                MouseButton::Left => mouse::Button::Left,
                MouseButton::Middle => mouse::Button::Middle,
                MouseButton::Right => mouse::Button::Right,
                MouseButton::WheelUp => mouse::Button::Four,
                MouseButton::WheelDown => mouse::Button::Five,
                MouseButton::WheelLeft => mouse::Button::Six,
                MouseButton::WheelRight => mouse::Button::Seven,
                MouseButton::Back => mouse::Button::Eight,
                MouseButton::Forward => mouse::Button::Nine,
            }))
            .set_mods(event.mods.ghostty())
            .set_position(Position {
                x: event.x,
                y: event.y,
            });
        state
            .encoder
            .set_options_from_terminal(&self.inner)
            .set_size(EncoderSize {
                screen_width: cols * cell.width,
                screen_height: rows * cell.height,
                cell_width: cell.width,
                cell_height: cell.height,
                padding_top: 0,
                padding_bottom: 0,
                padding_right: 0,
                padding_left: 0,
            })
            .set_any_button_pressed(event.buttons_held);
        let start = out.len();
        state.encoder.encode_to_vec(&state.event, out)?;
        let report = &out[start..];
        if report.is_empty() {
            return Ok(());
        }
        // Motion that encodes the same as the last report stayed in its cell
        // with the same buttons and modifiers. In the pixel format every move
        // encodes differently, so each one is reported.
        if event.action == MouseAction::Motion && report == state.last.as_slice() {
            out.truncate(start);
            return Ok(());
        }
        state.last.clear();
        state.last.extend_from_slice(report);
        Ok(())
    }
}
