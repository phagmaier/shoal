use libghostty_vt::{
    fmt::{Format, Formatter, FormatterOptions},
    screen::Screen,
    selection::Selection,
    terminal::{ModeKind, Point, PointCoordinate},
};

/// The continuation cap every live terminal tracks (M1 spec §4.2).
pub const CONTINUATION_BYTES: usize = 4096;

/// A libghostty failure. The cause is kept opaque so no other crate depends on
/// libghostty's types.
#[derive(Debug, Clone, thiserror::Error)]
#[error("terminal: {0}")]
pub struct TermError(pub(crate) libghostty_vt::Error);

impl From<libghostty_vt::Error> for TermError {
    fn from(error: libghostty_vt::Error) -> Self {
        Self(error)
    }
}

/// Where a terminal's answers to program queries go: the pane's PTY.
pub type ReplySink = Box<dyn FnMut(&[u8])>;

/// A terminal mode by number, as in `CSI ? <n> h` (DEC) or `CSI <n> h` (ANSI).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    Dec(u16),
    Ansi(u16),
}

/// What [`Terminal::capture`] produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureFormat {
    /// Plain text, one line per row.
    Text,
    /// Terminal bytes that redraw the content with its styles.
    Vt,
}

/// One terminal emulator: the daemon's pane terminal, a window's mirror, or a
/// scratch copy. Not `Send`: it lives on the thread that made it.
///
/// Only a terminal made by [`Terminal::with_replies`] answers program queries.
/// Mirrors from [`Terminal::restore`] and terminals from [`Terminal::new`] drop
/// every reply, so only the daemon's pane terminal ever writes to a PTY.
#[derive(Debug)]
pub struct Terminal {
    pub(crate) inner: libghostty_vt::Terminal<'static, 'static>,
    pub(crate) view: View,
}

/// What only a drawn terminal needs, made on first use: its single render
/// state and its input encoders.
#[derive(Debug, Default)]
pub(crate) struct View {
    pub(crate) render: Option<crate::render::Renderer>,
    pub(crate) keys: Option<crate::input::KeyEncoder>,
    pub(crate) mouse: Option<crate::mouse::MouseEncoder>,
}

impl Terminal {
    pub(crate) fn from_inner(inner: libghostty_vt::Terminal<'static, 'static>) -> Self {
        Self {
            inner,
            view: View::default(),
        }
    }

    /// A terminal that answers nothing, keeping up to `scrollback` lines of
    /// history and tracking up to [`CONTINUATION_BYTES`] of unfinished input.
    pub fn new(cols: u16, rows: u16, scrollback: u32) -> Result<Self, TermError> {
        let mut terminal = Self::from_inner(libghostty_vt::Terminal::new(cols, rows)?);
        terminal.set_scrollback(scrollback)?;
        terminal
            .inner
            .set_continuation_max_bytes(CONTINUATION_BYTES)?;
        Ok(terminal)
    }

    /// The pane terminal: like [`Terminal::new`], plus every reply libghostty
    /// generates (cursor position, mode reports, ...) goes to `replies`.
    pub fn with_replies(
        cols: u16,
        rows: u16,
        scrollback: u32,
        mut replies: ReplySink,
    ) -> Result<Self, TermError> {
        let mut terminal = Self::new(cols, rows, scrollback)?;
        terminal
            .inner
            .on_pty_write(move |_, bytes| replies(bytes))?;
        Ok(terminal)
    }

    /// Change the history line limit; zero erases history and disables retention.
    pub fn set_scrollback(&mut self, lines: u32) -> Result<(), TermError> {
        let lines = usize::try_from(lines).expect("invariant: u32 fits in usize on Linux");
        self.inner
            .set_scrollback_max_lines(Some(lines))?
            .set_scrollback_max_bytes((lines == 0).then_some(0))?;
        Ok(())
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.inner.vt_write(bytes);
    }

    /// Feed only up to the first point where the parser is at ground. Returns
    /// how many bytes that took (0 if already at ground), or `None` if all of
    /// `bytes` was fed without reaching ground.
    pub fn feed_until_ground(&mut self, bytes: &[u8]) -> Result<Option<usize>, TermError> {
        Ok(self.inner.vt_write_until_ground(bytes)?)
    }

    /// Whether the parser is outside every escape sequence and UTF-8 character.
    pub fn is_ground(&self) -> Result<bool, TermError> {
        Ok(self.inner.is_vt_ground()?)
    }

    /// Resize, reflowing the primary screen. Pixel sizes are unknown to Shoal's
    /// protocol (cells only), so pixel size reports say zero.
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), TermError> {
        Ok(self.inner.resize(cols, rows, 0, 0)?)
    }

    pub fn cols(&self) -> Result<u16, TermError> {
        Ok(self.inner.cols()?)
    }

    pub fn rows(&self) -> Result<u16, TermError> {
        Ok(self.inner.rows()?)
    }

    /// The cursor's column and row in the active area, from zero.
    pub fn cursor(&self) -> Result<(u16, u16), TermError> {
        Ok((self.inner.cursor_x()?, self.inner.cursor_y()?))
    }

    pub fn mode(&self, mode: Mode) -> Result<bool, TermError> {
        let mode = match mode {
            Mode::Dec(number) => libghostty_vt::terminal::Mode::new(number, ModeKind::Dec),
            Mode::Ansi(number) => libghostty_vt::terminal::Mode::new(number, ModeKind::Ansi),
        };
        Ok(self.inner.mode(mode)?)
    }

    /// Whether a full-screen program's alternate screen is showing, whichever of
    /// modes 47, 1047 or 1049 entered it.
    pub fn alternate_screen(&self) -> Result<bool, TermError> {
        Ok(self.inner.active_screen()? == Screen::Alternate)
    }

    /// Rows of history above the active screen.
    pub fn scrollback_rows(&self) -> Result<usize, TermError> {
        Ok(self.inner.scrollback_rows()?)
    }

    /// The title a program set with OSC 0 or 2, as raw bytes.
    pub fn title(&self) -> Result<Vec<u8>, TermError> {
        Ok(self.inner.title()?.to_vec())
    }

    /// The directory a program reported with OSC 7, as raw bytes.
    pub fn pwd(&self) -> Result<Vec<u8>, TermError> {
        Ok(self.inner.pwd()?.to_vec())
    }

    /// The active screen's content, with its history when `scrollback` is set
    /// (`pane.capture`, M1 spec §5.4).
    pub fn capture(&self, format: CaptureFormat, scrollback: bool) -> Result<Vec<u8>, TermError> {
        let format = match format {
            CaptureFormat::Text => Format::Plain,
            CaptureFormat::Vt => Format::Vt,
        };
        let options = FormatterOptions::new()
            .with_format(format)
            .with_unwrap(false)
            .with_trim(true)
            .with_style(true)
            .with_hyperlink(true);
        if scrollback {
            return self.format(options);
        }
        let corner = |x, y| self.inner.grid_ref(Point::Active(PointCoordinate { x, y }));
        let (cols, rows) = (self.cols()?, self.rows()?);
        let screen = Selection::new(corner(0, 0)?, corner(cols - 1, u32::from(rows) - 1)?, false);
        self.format(options.with_selection(&screen))
    }

    pub(crate) fn format(&self, options: FormatterOptions<'_, '_>) -> Result<Vec<u8>, TermError> {
        let mut formatter = Formatter::new(&self.inner, options)?;
        Ok(formatter.format_alloc(None)?.to_vec())
    }
}
