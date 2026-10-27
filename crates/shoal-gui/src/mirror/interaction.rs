//! Viewport state belongs to the retained mirror, independently of focus.

use super::Mirror;
use crate::links::{self, Link};
use shoal_term::{CellPoint, Scroll, TermError};

impl Mirror {
    pub fn scroll(&mut self, scroll: Scroll) -> Result<(), TermError> {
        self.terminal.scroll(scroll);
        self.fraction = 0.0;
        if self.terminal.viewport()?.at_bottom {
            self.new_output = false;
        }
        Ok(())
    }

    /// Positive deltas move down. Keep the integral position in the core;
    /// paint the fractional part with its one row of overscan.
    pub fn scroll_rows(&mut self, delta: f32) -> Result<(), TermError> {
        if !delta.is_finite() {
            return Ok(());
        }
        let view = self.terminal.viewport()?;
        let target = (view.offset as f64 + f64::from(self.fraction) + f64::from(delta))
            .clamp(0.0, view.history as f64);
        self.terminal
            .scroll(Scroll::Rows(target.floor() as i32 - view.offset as i32));
        self.fraction = target.fract() as f32;
        if self.terminal.viewport()?.at_bottom {
            self.new_output = false;
        }
        Ok(())
    }

    pub fn scroll_page(&mut self, direction: i32) -> Result<(), TermError> {
        self.scroll(Scroll::Rows(
            i32::from(self.terminal.viewport()?.rows) * direction,
        ))
    }

    pub fn fraction(&self) -> f32 {
        self.fraction
    }
    pub fn has_new_output(&self) -> bool {
        self.new_output
    }

    pub fn begin_selection(&mut self, at: CellPoint, clicks: usize) -> Result<(), TermError> {
        self.terminal.begin_selection(at, clicks)
    }
    pub fn extend_selection(&mut self, at: CellPoint) -> Result<(), TermError> {
        self.terminal.extend_selection(at)
    }
    pub fn selection_text(&self) -> Result<String, TermError> {
        self.terminal.selection_text()
    }
    pub fn clear_selection(&mut self) -> Result<(), TermError> {
        self.terminal.clear_selection()
    }

    pub fn links(&self, row: u16) -> Result<Vec<Link>, TermError> {
        let cells = self.terminal.row_text(row)?;
        let line = self.terminal.logical_line(row)?;
        let text = line.cells.concat();
        let mut boundaries = Vec::with_capacity(line.cells.len());
        let mut byte = 0;
        for cell in &line.cells {
            boundaries.push(byte);
            byte += cell.len();
        }
        let mut found = Vec::<Link>::new();
        for col in 0..cells.len() as u16 {
            if let Some(uri) = self.terminal.hyperlink(CellPoint { row, col })?
                && links::is_uri(&uri)
            {
                match found.last_mut() {
                    Some(last) if last.uri == uri && last.columns.end == col => {
                        last.columns.end += 1
                    }
                    _ => found.push(Link {
                        columns: col..col + 1,
                        uri,
                    }),
                }
            }
        }
        for range in links::urls(&text) {
            let start = boundaries
                .iter()
                .position(|b| *b == range.start)
                .unwrap_or(0)
                .max(line.hovered.start);
            let end = boundaries
                .iter()
                .rposition(|b| *b < range.end)
                .map_or(start, |c| c + 1)
                .min(line.hovered.end);
            if start >= end {
                continue;
            }
            let (start, end) = (
                (start - line.hovered.start) as u16,
                (end - line.hovered.start) as u16,
            );
            if !found
                .iter()
                .any(|link| link.columns.start < end && start < link.columns.end)
            {
                found.push(Link {
                    columns: start..end,
                    uri: text[range].into(),
                });
            }
        }
        Ok(found)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        draw,
        mirror::{Next, Setup, State},
    };
    use shoal_client::{StreamItem, model::GridSize};
    use shoal_term::{CursorShape, Terminal};

    #[test]
    fn fractional_scroll_preserves_view_and_selection_until_returning_to_output() {
        let mut source = Terminal::new(12, 3, 100).unwrap();
        source.feed(b"zero\r\none\r\ntwo\r\nthree\r\nfour\r\nfive\r\nsix");
        let setup = Setup {
            scrollback: 100,
            cursor: CursorShape::Block,
            blink: false,
            ..Setup::from_config(
                &shoal_config::Config::default(),
                &shoal_config::LoadedTheme::default().palette(&shoal_config::Config::default()),
            )
        };
        let mut state = State::new();
        state.apply(
            StreamItem::Snapshot {
                seq: 1,
                size: GridSize { cols: 12, rows: 3 },
                bytes: source.snapshot().unwrap(),
            },
            &setup,
        );
        let settings = draw::Settings {
            cursor: shoal_config::schema::Rgb([0, 0, 0]),
            focused: true,
        };
        let State::Live(mirror) = &mut state else {
            panic!()
        };
        mirror.scroll_rows(-1.25).unwrap();
        assert_eq!(mirror.terminal.viewport().unwrap().offset, 2);
        assert_eq!(mirror.fraction(), 0.75);
        mirror
            .begin_selection(CellPoint { row: 0, col: 1 }, 3)
            .unwrap();
        assert_eq!(mirror.selection_text().unwrap(), "two");
        let before = mirror.draw(&settings).unwrap();
        assert_eq!(
            before
                .rows
                .iter()
                .map(|r| r.text[0].text.as_str())
                .collect::<Vec<_>>(),
            ["two", "three", "four", "five"]
        );
        // "five" is the row below the three-row viewport that the fraction
        // scrolls in; painting clips to the viewport's rows.
        assert_eq!(before.visible_rows, 3);
        assert_eq!(before.selection, [(0, 0..12)]);
        assert_eq!(
            state.apply(
                StreamItem::Output {
                    seq: 2,
                    bytes: b"\r\nseven".to_vec()
                },
                &setup
            ),
            Next::Redraw
        );
        let State::Live(mirror) = &mut state else {
            panic!()
        };
        let mut reloaded = setup;
        reloaded.theme.foreground = shoal_term::Rgb {
            r: 12,
            g: 34,
            b: 56,
        };
        mirror.apply_setup(&reloaded).unwrap();
        mirror.invalidate_rows();
        let after = mirror.draw(&settings).unwrap();
        assert_eq!(
            after
                .rows
                .iter()
                .map(|r| r.text[0].text.as_str())
                .collect::<Vec<_>>(),
            ["two", "three", "four", "five"]
        );
        assert_eq!(after.fraction, 0.75);
        assert!(mirror.has_new_output());
        assert_eq!(mirror.selection_text().unwrap(), "two");
        mirror.scroll_rows(1000.0).unwrap();
        assert!(!mirror.has_new_output());
        assert_eq!(mirror.fraction(), 0.0);
        let bottom = mirror.draw(&settings).unwrap();
        assert_eq!((bottom.rows.len(), bottom.visible_rows), (3, 3));
    }

    #[test]
    fn osc8_takes_precedence_and_url_columns_follow_wide_unicode() {
        let mut source = Terminal::new(80, 2, 100).unwrap();
        source.feed("界 https://example.org/é \x1b]8;;https://target.test\x1b\\http://label.test\x1b]8;;\x1b\\".as_bytes());
        let mut mirror = Mirror::new(
            &source.snapshot().unwrap(),
            GridSize { cols: 80, rows: 2 },
            &Setup {
                scrollback: 100,
                cursor: CursorShape::Bar,
                blink: false,
                ..Setup::from_config(
                    &shoal_config::Config::default(),
                    &shoal_config::LoadedTheme::default().palette(&shoal_config::Config::default()),
                )
            },
        )
        .unwrap();
        let links = mirror.links(0).unwrap();
        assert_eq!(
            links,
            [
                Link {
                    columns: 25..42,
                    uri: "https://target.test".into()
                },
                Link {
                    columns: 3..24,
                    uri: "https://example.org/é".into()
                },
            ]
        );
        mirror.scroll(Scroll::Top).unwrap();
    }

    #[test]
    fn wrapped_urls_open_in_full_from_any_visible_fragment() {
        let mut source = Terminal::new(12, 2, 100).unwrap();
        source.feed(b"https://example.org/a_(b)\r\nlast");
        let mut mirror = Mirror::new(
            &source.snapshot().unwrap(),
            GridSize { cols: 12, rows: 2 },
            &Setup {
                scrollback: 100,
                cursor: CursorShape::Bar,
                blink: false,
                ..Setup::from_config(
                    &shoal_config::Config::default(),
                    &shoal_config::LoadedTheme::default().palette(&shoal_config::Config::default()),
                )
            },
        )
        .unwrap();
        assert_eq!(
            mirror.links(0).unwrap(),
            [Link {
                columns: 0..1,
                uri: "https://example.org/a_(b)".into()
            }]
        );
        mirror.scroll(Scroll::Top).unwrap();
        assert_eq!(
            mirror.links(0).unwrap(),
            [Link {
                columns: 0..12,
                uri: "https://example.org/a_(b)".into()
            }]
        );
    }
}
