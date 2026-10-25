//! Compare two terminals cell by cell, mode by mode, and on other state.

use libghostty_vt::terminal::{Mode, Point, PointCoordinate, Terminal};

type Term = Terminal<'static, 'static>;

pub const MODES: &[(&str, Mode)] = &[
    ("KAM", Mode::KAM),
    ("IRM insert", Mode::INSERT),
    ("SRM", Mode::SRM),
    ("LNM linefeed", Mode::LINEFEED),
    ("DECCKM", Mode::DECCKM),
    ("132 column", Mode::_132_COLUMN),
    ("slow scroll", Mode::SLOW_SCROLL),
    ("reverse colors", Mode::REVERSE_COLORS),
    ("origin", Mode::ORIGIN),
    ("wraparound", Mode::WRAPAROUND),
    ("autorepeat", Mode::AUTOREPEAT),
    ("X10 mouse", Mode::X10_MOUSE),
    ("cursor blinking", Mode::CURSOR_BLINKING),
    ("cursor visible", Mode::CURSOR_VISIBLE),
    ("enable mode 3", Mode::ENABLE_MODE3),
    ("reverse wrap", Mode::REVERSE_WRAP),
    ("alt screen 47", Mode::ALT_SCREEN_LEGACY),
    ("keypad keys", Mode::KEYPAD_KEYS),
    ("left/right margin", Mode::LEFT_RIGHT_MARGIN),
    ("normal mouse 1000", Mode::NORMAL_MOUSE),
    ("button mouse 1002", Mode::BUTTON_MOUSE),
    ("any mouse 1003", Mode::ANY_MOUSE),
    ("focus events 1004", Mode::FOCUS_EVENT),
    ("UTF-8 mouse 1005", Mode::UTF8_MOUSE),
    ("SGR mouse 1006", Mode::SGR_MOUSE),
    ("alt scroll 1007", Mode::ALT_SCROLL),
    ("urxvt mouse 1015", Mode::URXVT_MOUSE),
    ("SGR pixels mouse 1016", Mode::SGR_PIXELS_MOUSE),
    ("numlock keypad 1035", Mode::NUMLOCK_KEYPAD),
    ("alt esc prefix 1036", Mode::ALT_ESC_PREFIX),
    ("alt sends esc 1039", Mode::ALT_SENDS_ESC),
    ("reverse wrap ext 1045", Mode::REVERSE_WRAP_EXT),
    ("alt screen 1047", Mode::ALT_SCREEN),
    ("save cursor 1048", Mode::SAVE_CURSOR),
    ("alt screen save 1049", Mode::ALT_SCREEN_SAVE),
    ("bracketed paste 2004", Mode::BRACKETED_PASTE),
    ("sync output 2026", Mode::SYNC_OUTPUT),
    ("grapheme cluster 2027", Mode::GRAPHEME_CLUSTER),
    ("color scheme report 2031", Mode::COLOR_SCHEME_REPORT),
    ("visibility report 2033", Mode::VISIBILITY_REPORT),
    ("in-band resize 2048", Mode::IN_BAND_RESIZE),
    ("paste events 5522", Mode::PASTE_EVENTS),
];

#[derive(Default)]
pub struct Report {
    pub cell_mismatches: usize,
    pub cell_examples: Vec<String>,
    pub cursor: Vec<String>,
    pub modes: Vec<String>,
    pub state: Vec<String>,
}

impl Report {
    pub fn failed() -> Self {
        Report {
            state: vec!["snapshot encode or decode failed".into()],
            ..Report::default()
        }
    }

    pub fn is_match(&self) -> bool {
        self.cell_mismatches == 0
            && self.cursor.is_empty()
            && self.modes.is_empty()
            && self.state.is_empty()
    }

    pub fn summary(&self) -> String {
        if self.is_match() {
            return "match".into();
        }
        let mut parts = Vec::new();
        if self.cell_mismatches > 0 {
            parts.push(format!("{} cells differ", self.cell_mismatches));
        }
        if !self.cursor.is_empty() {
            let names: Vec<&str> = self
                .cursor
                .iter()
                .map(|c| c.split(':').next().unwrap_or(c))
                .collect();
            parts.push(format!("cursor: {}", names.join(", ")));
        }
        if !self.modes.is_empty() {
            parts.push(format!("modes: {}", self.modes.join(", ")));
        }
        if !self.state.is_empty() {
            let names: Vec<&str> = self
                .state
                .iter()
                .map(|c| c.split(':').next().unwrap_or(c))
                .collect();
            parts.push(format!("state: {}", names.join(", ")));
        }
        format!("**{}**", parts.join("; "))
    }

    pub fn details(&self) -> String {
        let mut out = String::new();
        for e in &self.cell_examples {
            out.push_str(&format!("- cell {e}\n"));
        }
        for e in self.cursor.iter().chain(&self.state) {
            out.push_str(&format!("- {e}\n"));
        }
        if !self.modes.is_empty() {
            out.push_str(&format!("- modes differ: {}\n", self.modes.join(", ")));
        }
        out
    }
}

fn diff<T: std::fmt::Debug + PartialEq>(out: &mut Vec<String>, name: &str, a: T, b: T) {
    if a != b {
        out.push(format!("{name}: original {a:?}, restored {b:?}"));
    }
}

#[derive(Debug, PartialEq)]
struct CellInfo {
    text: String,
    wide: String,
    style: String,
    link: String,
    protected: bool,
    row_wrapped: bool,
    row_prompt: String,
}

fn cell_info(t: &Term, x: u16, y: u32) -> Option<CellInfo> {
    let r = t.grid_ref(Point::Screen(PointCoordinate { x, y })).ok()?;
    let mut buf = ['\0'; 32];
    let n = r.graphemes(&mut buf).unwrap_or(0);
    let mut link = [0u8; 4096];
    let ln = r.hyperlink_uri(&mut link).unwrap_or(0);
    let cell = r.cell().ok()?;
    let row = r.row().ok()?;
    Some(CellInfo {
        text: buf[..n].iter().collect(),
        wide: format!("{:?}", cell.wide().ok()),
        style: format!("{:?}", r.style().ok()),
        link: String::from_utf8_lossy(&link[..ln]).into_owned(),
        protected: cell.is_protected().unwrap_or(false),
        row_wrapped: row.is_wrapped().unwrap_or(false),
        row_prompt: format!("{:?}", row.semantic_prompt().ok()),
    })
}

fn field_diff(a: Option<&CellInfo>, b: Option<&CellInfo>) -> String {
    let (Some(a), Some(b)) = (a, b) else {
        return format!("original {a:?}, restored {b:?}");
    };
    let mut out = Vec::new();
    let mut f = |name: &str, x: String, y: String| {
        if x != y {
            out.push(format!("{name} {x} -> {y}"));
        }
    };
    f("text", format!("{:?}", a.text), format!("{:?}", b.text));
    f("wide", a.wide.clone(), b.wide.clone());
    f("style", a.style.clone(), b.style.clone());
    f("link", format!("{:?}", a.link), format!("{:?}", b.link));
    f(
        "protected",
        a.protected.to_string(),
        b.protected.to_string(),
    );
    f(
        "row wrapped",
        a.row_wrapped.to_string(),
        b.row_wrapped.to_string(),
    );
    f("row prompt", a.row_prompt.clone(), b.row_prompt.clone());
    out.join("; ")
}

pub fn compare(a: &Term, b: &Term) -> Report {
    let mut r = Report::default();

    // Geometry and screen.
    diff(&mut r.state, "cols", a.cols().ok(), b.cols().ok());
    diff(&mut r.state, "rows", a.rows().ok(), b.rows().ok());
    diff(
        &mut r.state,
        "active screen",
        a.active_screen().ok(),
        b.active_screen().ok(),
    );
    diff(
        &mut r.state,
        "total rows",
        a.total_rows().ok(),
        b.total_rows().ok(),
    );
    diff(
        &mut r.state,
        "scrollback rows",
        a.scrollback_rows().ok(),
        b.scrollback_rows().ok(),
    );

    // Cells: the whole active screen including scrollback, aligned from the bottom
    // so a scrollback length difference shows up once, not on every row.
    let cols = a.cols().unwrap_or(0).min(b.cols().unwrap_or(0));
    let ta = a.total_rows().unwrap_or(0) as u32;
    let tb = b.total_rows().unwrap_or(0) as u32;
    let rows = ta.min(tb);
    for i in 0..rows {
        let (ya, yb) = (ta - rows + i, tb - rows + i);
        for x in 0..cols {
            let ca = cell_info(a, x, ya);
            let cb = cell_info(b, x, yb);
            if ca != cb {
                r.cell_mismatches += 1;
                if r.cell_examples.len() < 4 {
                    r.cell_examples.push(format!(
                        "row {ya} col {x} (of {ta} rows): {}",
                        field_diff(ca.as_ref(), cb.as_ref())
                    ));
                }
            }
        }
    }

    // Cursor.
    diff(&mut r.cursor, "x", a.cursor_x().ok(), b.cursor_x().ok());
    diff(&mut r.cursor, "y", a.cursor_y().ok(), b.cursor_y().ok());
    diff(
        &mut r.cursor,
        "pending wrap",
        a.is_cursor_pending_wrap().ok(),
        b.is_cursor_pending_wrap().ok(),
    );
    diff(
        &mut r.cursor,
        "visible",
        a.is_cursor_visible().ok(),
        b.is_cursor_visible().ok(),
    );
    diff(
        &mut r.cursor,
        "pen style",
        a.cursor_style().ok(),
        b.cursor_style().ok(),
    );
    diff(
        &mut r.cursor,
        "at prompt",
        a.is_cursor_at_prompt().ok(),
        b.is_cursor_at_prompt().ok(),
    );

    // Modes.
    for (name, mode) in MODES {
        if a.mode(*mode).ok() != b.mode(*mode).ok() {
            r.modes.push((*name).to_owned());
        }
    }

    // Other state.
    diff(
        &mut r.state,
        "kitty keyboard flags",
        a.kitty_keyboard_flags().ok(),
        b.kitty_keyboard_flags().ok(),
    );
    diff(
        &mut r.state,
        "mouse tracking",
        a.is_mouse_tracking().ok(),
        b.is_mouse_tracking().ok(),
    );
    diff(
        &mut r.state,
        "mouse shape",
        a.mouse_shape().ok(),
        b.mouse_shape().ok(),
    );
    diff(
        &mut r.state,
        "title",
        a.title().ok().map(<[u8]>::to_vec),
        b.title().ok().map(<[u8]>::to_vec),
    );
    diff(
        &mut r.state,
        "pwd",
        a.pwd().ok().map(<[u8]>::to_vec),
        b.pwd().ok().map(<[u8]>::to_vec),
    );
    diff(
        &mut r.state,
        "fg color (OSC 10)",
        a.fg_color().ok(),
        b.fg_color().ok(),
    );
    diff(
        &mut r.state,
        "bg color (OSC 11)",
        a.bg_color().ok(),
        b.bg_color().ok(),
    );
    diff(
        &mut r.state,
        "cursor color (OSC 12)",
        a.cursor_color().ok(),
        b.cursor_color().ok(),
    );
    let pa = a.color_palette().ok().map(|p| p.0);
    let pb = b.color_palette().ok().map(|p| p.0);
    if pa != pb {
        let changed = match (pa, pb) {
            (Some(pa), Some(pb)) => (0..256).filter(|i| pa[*i] != pb[*i]).count(),
            _ => 256,
        };
        r.state
            .push(format!("palette (OSC 4): {changed} entries differ"));
    }
    r
}
