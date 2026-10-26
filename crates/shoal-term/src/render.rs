//! Render-state access for drawing a terminal (M1 spec §7.3).
//!
//! Each terminal owns at most one libghostty render state, made on its first
//! [`Terminal::frame`]. A second render state would consume the first one's
//! dirty flags, so none can be made from outside this crate.

use std::collections::HashMap;

use libghostty_vt::{
    render::{CellIterator, CursorVisualStyle, RenderState, RowIteration, RowIterator, Snapshot},
    screen::{CellContentTag, CellWide},
    style::{self, PaletteIndex, RgbColor, StyleColor},
    terminal::CursorStyle,
};

use crate::{TermError, Terminal};

/// A color as red, green and blue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl From<RgbColor> for Rgb {
    fn from(RgbColor { r, g, b }: RgbColor) -> Self {
        Self { r, g, b }
    }
}

/// How much of the grid changed since the last [`Frame::mark_clean`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dirty {
    Clean,
    /// Some rows changed; [`Rows::Dirty`] reads just those.
    Partial,
    /// Everything must be redrawn; [`Rows::Dirty`] reads every row.
    Full,
}

/// Which rows [`Frame::read_rows`] reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rows {
    Dirty,
    All,
}

/// The terminal's current default colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Colors {
    pub background: Rgb,
    pub foreground: Rgb,
    /// The cursor color a program set (OSC 12), if any.
    pub cursor: Option<Rgb>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorShape {
    Block,
    Bar,
    Underline,
    HollowBlock,
}

impl Terminal {
    /// The cursor shown until a program chooses one (DECSCUSR), and again
    /// when it asks for the default (`CSI 0 SP q`) or resets. A shape the
    /// program chose, even before a snapshot, stays. `HollowBlock` is drawn
    /// for unfocused panes and isn't a default; it means `Block` here.
    pub fn set_default_cursor(&mut self, shape: CursorShape, blink: bool) -> Result<(), TermError> {
        let style = match shape {
            CursorShape::Block | CursorShape::HollowBlock => CursorStyle::Block,
            CursorShape::Bar => CursorStyle::Bar,
            CursorShape::Underline => CursorStyle::Underline,
        };
        self.inner
            .set_default_cursor_style(Some(style))?
            .set_default_cursor_blink(Some(blink))?;
        Ok(())
    }
}

/// Where the cursor is on the viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CursorPosition {
    pub col: u16,
    pub row: u16,
    /// The cursor is on the right half of a wide character, which starts at
    /// `col - 1`.
    pub wide_tail: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    /// `None` when the cursor is outside the viewport.
    pub position: Option<CursorPosition>,
    pub visible: bool,
    pub blinking: bool,
    pub shape: CursorShape,
}

/// Identifies a row as it moves: the same row keeps its id when the screen
/// scrolls, and ids are never reused. A row whose id is present and whose
/// `dirty` flag is clear can reuse cached drawing work.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RowId(libghostty_vt::render::RowId);

/// How many columns a cell's content takes, as libghostty reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellWidth {
    Narrow,
    /// The first column of a two-column character.
    Wide,
    /// The second column of a wide character; it draws nothing itself.
    SpacerTail,
    /// The last column of a row whose wide character wrapped to the next
    /// row; it draws nothing.
    SpacerHead,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Underline {
    #[default]
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

/// A cell's colors and attributes. Colors are resolved through the palette;
/// `None` means the terminal's default color. `inverse` is reported, not
/// applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[expect(clippy::struct_excessive_bools, reason = "independent SGR attributes")]
pub struct CellStyle {
    pub fg: Option<Rgb>,
    pub bg: Option<Rgb>,
    /// `None` means the underline takes the text color.
    pub underline_color: Option<Rgb>,
    pub bold: bool,
    pub italic: bool,
    pub faint: bool,
    pub blink: bool,
    pub inverse: bool,
    pub invisible: bool,
    pub strikethrough: bool,
    pub overline: bool,
    pub underline: Underline,
}

/// One column of a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub width: CellWidth,
    /// Index into [`RowCells::styles`]. Within one row, equal indices mean
    /// equal styles and different indices mean different styles.
    pub style: u16,
    text: (u32, u32),
}

/// The cells of one row, read in bulk. The frame reuses one buffer from row
/// to row, so reading allocates only when a row needs more room than any
/// before it.
#[derive(Debug, Clone)]
pub struct RowCells {
    y: u16,
    id: Option<RowId>,
    dirty: bool,
    cells: Vec<Cell>,
    styles: Vec<CellStyle>,
    /// Each style's index in `styles`, so a row of many colors stays linear.
    indices: HashMap<CellStyle, u16>,
    text: String,
}

impl RowCells {
    fn new() -> Self {
        Self {
            y: 0,
            id: None,
            dirty: false,
            cells: Vec::new(),
            styles: vec![CellStyle::default()],
            indices: HashMap::from([(CellStyle::default(), 0)]),
            text: String::new(),
        }
    }

    /// The row's position on the viewport, from zero at the top.
    pub fn y(&self) -> u16 {
        self.y
    }

    pub fn id(&self) -> RowId {
        self.id
            .expect("invariant: a row is only shown to callers after it is read")
    }

    /// Whether the row changed since the last [`Frame::mark_clean`].
    pub fn dirty(&self) -> bool {
        self.dirty
    }

    /// One cell per column.
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// The row's distinct styles. The first is always the default style.
    pub fn styles(&self) -> &[CellStyle] {
        &self.styles
    }

    pub fn style(&self, cell: &Cell) -> &CellStyle {
        &self.styles[usize::from(cell.style)]
    }

    /// The cell's grapheme cluster: empty for a blank cell, one character
    /// for most cells, several codepoints for a cluster such as an emoji
    /// sequence, which is drawn as one unit.
    pub fn text(&self, cell: &Cell) -> &str {
        let (start, end) = cell.text;
        &self.text[start as usize..end as usize]
    }

    fn clear(&mut self) {
        self.cells.clear();
        self.styles.truncate(1);
        self.indices.retain(|_, index| *index == 0);
        self.text.clear();
    }

    fn style_index(&mut self, style: CellStyle) -> u16 {
        *self.indices.entry(style).or_insert_with(|| {
            self.styles.push(style);
            u16::try_from(self.styles.len() - 1)
                .expect("invariant: a row has at most 1000 cells, so as many styles")
        })
    }
}

pub(crate) struct Renderer {
    state: RenderState<'static>,
    rows: RowIterator<'static>,
    cells: CellIterator<'static>,
    row: RowCells,
    scratch: String,
}

impl std::fmt::Debug for Renderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Renderer")
    }
}

impl Renderer {
    fn new() -> Result<Self, TermError> {
        Ok(Self {
            state: RenderState::new()?,
            rows: RowIterator::new()?,
            cells: CellIterator::new()?,
            row: RowCells::new(),
            scratch: String::new(),
        })
    }
}

/// What to draw now: the terminal's state at [`Terminal::frame`].
///
/// Dirty flags stay set until [`Frame::mark_clean`], so a frame dropped
/// without being drawn is drawn in full by the next one.
pub struct Frame<'t> {
    snapshot: Snapshot<'static, 't>,
    rows: &'t mut RowIterator<'static>,
    cells: &'t mut CellIterator<'static>,
    row: &'t mut RowCells,
    scratch: &'t mut String,
}

impl Terminal {
    /// Capture the terminal for drawing. The terminal's single render state
    /// is made on first use, so a terminal that is never drawn, such as the
    /// daemon's, never has one.
    pub fn frame(&mut self) -> Result<Frame<'_>, TermError> {
        if self.view.render.is_none() {
            self.view.render = Some(Renderer::new()?);
        }
        let Renderer {
            state,
            rows,
            cells,
            row,
            scratch,
        } = self
            .view
            .render
            .as_mut()
            .expect("invariant: the renderer was just made");
        Ok(Frame {
            snapshot: state.update(&self.inner)?,
            rows,
            cells,
            row,
            scratch,
        })
    }
}

impl Frame<'_> {
    pub fn dirty(&self) -> Result<Dirty, TermError> {
        Ok(match self.snapshot.dirty()? {
            libghostty_vt::render::Dirty::Clean => Dirty::Clean,
            libghostty_vt::render::Dirty::Partial => Dirty::Partial,
            libghostty_vt::render::Dirty::Full => Dirty::Full,
        })
    }

    /// Columns and rows.
    pub fn size(&self) -> Result<(u16, u16), TermError> {
        Ok((self.snapshot.cols()?, self.snapshot.rows()?))
    }

    pub fn colors(&self) -> Result<Colors, TermError> {
        let colors = self.snapshot.colors()?;
        Ok(Colors {
            background: colors.background.into(),
            foreground: colors.foreground.into(),
            cursor: colors.cursor.map(Rgb::from),
        })
    }

    pub fn cursor(&self) -> Result<Cursor, TermError> {
        let cursor = self.snapshot.cursor()?;
        Ok(Cursor {
            position: cursor.viewport.map(|at| CursorPosition {
                col: at.x,
                row: at.y,
                wide_tail: at.at_wide_tail,
            }),
            visible: cursor.visible,
            blinking: cursor.blinking,
            shape: match cursor.visual_style {
                CursorVisualStyle::Block => CursorShape::Block,
                CursorVisualStyle::Bar => CursorShape::Bar,
                CursorVisualStyle::Underline => CursorShape::Underline,
                CursorVisualStyle::BlockHollow => CursorShape::HollowBlock,
                // A shape added in a later pin still marks the cursor cell.
                _ => CursorShape::Block,
            },
        })
    }

    /// Read rows from top to bottom, calling `each` with every row read.
    pub fn read_rows(
        &mut self,
        which: Rows,
        mut each: impl FnMut(&RowCells),
    ) -> Result<(), TermError> {
        let palette = self.snapshot.colors()?.palette;
        let mut rows = self.rows.update(&self.snapshot)?;
        loop {
            let row = match which {
                Rows::Dirty => rows.next_dirty().map(|(y, row)| (Some(y), row)),
                Rows::All => rows.next().map(|row| (None, row)),
            };
            let Some((y, row)) = row else {
                return Ok(());
            };
            let y = match y {
                Some(y) => y,
                None => u16::try_from(row.viewport_y()?)
                    .expect("invariant: without overscan every row is on the viewport"),
            };
            read_row(self.cells, row, &palette, self.scratch, y, self.row)?;
            each(self.row);
        }
    }

    /// Mark the frame drawn: the frame and every row become clean.
    pub fn mark_clean(self) -> Result<(), TermError> {
        Ok(self.snapshot.clean()?)
    }
}

/// Read one row. Each accessor is an FFI call, so this reads raw cells in
/// bulk and asks for a style only when the row is styled and the style id
/// changes (ids compare within a row), as the T00 spike measured.
fn read_row(
    cells: &mut CellIterator<'static>,
    row: &RowIteration<'static, '_>,
    palette: &[RgbColor; 256],
    scratch: &mut String,
    y: u16,
    out: &mut RowCells,
) -> Result<(), TermError> {
    out.clear();
    out.y = y;
    out.id = Some(RowId(row.id()?));
    out.dirty = row.dirty()?;
    let flags = row.raw_row()?;
    let styled = flags.is_styled()?;
    let mut iter = cells.update(row)?;
    let mut last_style: Option<(style::Id, u16)> = None;
    for (x, raw) in row.cells_raw()?.enumerate() {
        let col = u16::try_from(x).expect("invariant: a row has at most 1000 cells");
        let width = match raw.wide()? {
            CellWide::Narrow => CellWidth::Narrow,
            CellWide::Wide => CellWidth::Wide,
            CellWide::SpacerTail => CellWidth::SpacerTail,
            CellWide::SpacerHead => CellWidth::SpacerHead,
        };
        let start = text_offset(&out.text);
        let tag = raw.content_tag()?;
        let text_cell = match tag {
            CellContentTag::Codepoint => {
                if let Some(c) = char::from_u32(raw.codepoint()?).filter(|c| *c != '\0') {
                    out.text.push(c);
                }
                true
            }
            CellContentTag::CodepointGrapheme => {
                iter.select(col)?;
                iter.graphemes_utf8(scratch)?;
                out.text.push_str(scratch);
                true
            }
            CellContentTag::BgColorPalette | CellContentTag::BgColorRgb => false,
        };
        let style = if text_cell && !styled {
            0
        } else {
            let id = raw.style_id()?;
            match last_style {
                Some((last, index)) if text_cell && last == id => index,
                _ => {
                    iter.select(col)?;
                    let index = out.style_index(cell_style(&iter, palette)?);
                    if text_cell {
                        last_style = Some((id, index));
                    }
                    index
                }
            }
        };
        out.cells.push(Cell {
            width,
            style,
            text: (start, text_offset(&out.text)),
        });
    }
    Ok(())
}

fn text_offset(text: &str) -> u32 {
    u32::try_from(text.len()).expect("invariant: a row's text is far below 4 GiB")
}

fn cell_style(
    cell: &libghostty_vt::render::CellIteration<'static, '_>,
    palette: &[RgbColor; 256],
) -> Result<CellStyle, TermError> {
    let style = cell.style()?;
    let underline_color = match style.underline_color {
        StyleColor::None => None,
        StyleColor::Palette(PaletteIndex(index)) => Some(palette[usize::from(index)].into()),
        StyleColor::Rgb(rgb) => Some(rgb.into()),
    };
    Ok(CellStyle {
        fg: cell.fg_color()?.map(Rgb::from),
        bg: cell.bg_color()?.map(Rgb::from),
        underline_color,
        bold: style.bold,
        italic: style.italic,
        faint: style.faint,
        blink: style.blink,
        inverse: style.inverse,
        invisible: style.invisible,
        strikethrough: style.strikethrough,
        overline: style.overline,
        underline: match style.underline {
            style::Underline::Single => Underline::Single,
            style::Underline::Double => Underline::Double,
            style::Underline::Curly => Underline::Curly,
            style::Underline::Dotted => Underline::Dotted,
            style::Underline::Dashed => Underline::Dashed,
            style::Underline::None => Underline::None,
            // A kind added in a later pin is still an underline.
            _ => Underline::Single,
        },
    })
}
