//! A selection in the visible grid. T45 adds history and content anchors.
//! Empty cell strings are wide-character tails; blank cells contain a space.

use shoal_model::PaneId;
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CellPoint {
    pub row: u16,
    pub col: u16,
}

#[derive(Debug)]
pub struct Selection {
    pub pane: PaneId,
    pub dragging: bool,
    screen: Vec<Vec<String>>,
    anchor: CellPoint,
    end: CellPoint,
    clicks: usize,
    moved: bool,
}

impl Selection {
    pub fn new(pane: PaneId, screen: Vec<Vec<String>>, at: CellPoint, clicks: usize) -> Self {
        Self {
            pane,
            dragging: true,
            screen,
            anchor: at,
            end: at,
            clicks,
            moved: clicks >= 2,
        }
    }

    pub fn move_to(&mut self, at: CellPoint) {
        self.moved |= at != self.anchor;
        self.end = at;
    }

    fn edge(&self, point: CellPoint) -> Range<u16> {
        let Some(row) = self.screen.get(usize::from(point.row)) else {
            return 0..0;
        };
        if row.is_empty() {
            return 0..0;
        }
        let mut col = usize::from(point.col).min(row.len() - 1);
        while col > 0 && row[col].is_empty() {
            col -= 1;
        }
        let mut start = col;
        let mut end = col + 1;
        while end < row.len() && row[end].is_empty() {
            end += 1;
        }
        if self.clicks >= 3 {
            return 0..row.len() as u16;
        }
        if self.clicks == 2 {
            let class = |text: &str| match text.chars().next() {
                Some(c) if c.is_alphanumeric() || c == '_' => 1,
                Some(c) if c.is_whitespace() => 2,
                _ => 3,
            };
            let kind = class(&row[col]);
            while start > 0 && (row[start - 1].is_empty() || class(&row[start - 1]) == kind) {
                start -= 1;
            }
            while end < row.len() && (row[end].is_empty() || class(&row[end]) == kind) {
                end += 1;
            }
        }
        start as u16..end as u16
    }

    pub fn span(&self, row: u16) -> Option<Range<u16>> {
        if !self.moved {
            return None;
        }
        let (a, b) = if self.anchor <= self.end {
            (self.anchor, self.end)
        } else {
            (self.end, self.anchor)
        };
        if row < a.row || row > b.row {
            return None;
        }
        let start = if row == a.row { self.edge(a).start } else { 0 };
        let end = if row == b.row {
            self.edge(b).end
        } else {
            self.screen.get(usize::from(row))?.len() as u16
        };
        (start < end).then_some(start..end)
    }

    pub fn text(&self) -> String {
        self.screen
            .iter()
            .enumerate()
            .filter_map(|(y, row)| {
                let span = self.span(y as u16)?;
                let text = row[usize::from(span.start)..usize::from(span.end)].concat();
                Some(text.trim_end_matches(' ').to_owned())
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn point(row: u16, col: u16) -> CellPoint {
        CellPoint { row, col }
    }
    fn selection(at: CellPoint, clicks: usize) -> Selection {
        let screen = vec![
            vec!["a", "界", "", "b", " ", " "],
            vec!["c", "a", "f", "é", " ", " "],
        ]
        .into_iter()
        .map(|r| r.into_iter().map(str::to_owned).collect())
        .collect();
        Selection::new(PaneId::new(1), screen, at, clicks)
    }
    #[test]
    fn reverse_drag_preserves_unicode_and_wide_cells() {
        let mut s = selection(point(1, 2), 1);
        assert_eq!(s.text(), "");
        s.move_to(point(0, 2));
        assert_eq!(s.text(), "界b\ncaf");
        assert_eq!(s.span(0), Some(1..6));
        assert_eq!(s.span(1), Some(0..3));
    }
    #[test]
    fn double_and_triple_click_select_words_and_lines() {
        assert_eq!(selection(point(1, 3), 2).text(), "café");
        assert_eq!(selection(point(0, 2), 3).text(), "a界b");
    }
}
