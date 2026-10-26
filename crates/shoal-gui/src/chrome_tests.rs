use serde_json::{Value, json};
use shoal_config::{Palette, schema::Rgb};
use shoal_model::Axis;

use super::*;

const BACKGROUND: Rgb = Rgb([0x10, 0x10, 0x10]);
const FOREGROUND: Rgb = Rgb([0xf0, 0xf0, 0xf0]);
const TOP_BAR: Rgb = Rgb([0x08, 0x08, 0x08]);
const ACCENT: Rgb = Rgb([0xff, 0x40, 0xa3]);
const LIGHTER: Rgb = Rgb([0x30, 0x30, 0x30]);
const INACTIVE: Rgb = Rgb([0x80, 0x80, 0x80]);

/// Distinct colors per role, so each assertion names the role it expects.
fn palette() -> Palette {
    Palette {
        background: BACKGROUND,
        foreground: FOREGROUND,
        cursor: Rgb([1, 1, 1]),
        selection_background: Rgb([2, 2, 2]),
        selection_foreground: Rgb([3, 3, 3]),
        ansi: [Rgb([4, 4, 4]); 16],
        top_bar: TOP_BAR,
        accent: ACCENT,
        unfocused_border: LIGHTER,
        inactive_text: INACTIVE,
    }
}

fn window() -> WindowConfig {
    WindowConfig::default()
}

fn pane(id: u64, title: &str, bell: bool) -> (String, Value) {
    (
        format!("p{id}"),
        json!({
            "id": format!("p{id}"), "kind": {"kind": "shell", "argv": []},
            "size": {"cols": 80, "rows": 24}, "cwd": null, "program": null, "title": title,
            "status": {"state": "running"}, "bell": bell, "restored_from": null,
        }),
    )
}

/// Session `work` (s1): t2 splits p3 | p5 side by side with p5 focused and
/// is active; t6 is named "logs" and holds p7, whose bell rang; t8 holds p9
/// and p11 with p11 zoomed. Session `other` (s12) holds t13 with p14.
fn workspace(active_tab: &str) -> Workspace {
    let session = |id: &str, name: &str, active: &str, tabs: Value| {
        json!({"id": id, "name": name, "active_tab": active, "tabs": tabs, "windows": 1,
               "created_at": "2026-10-25T00:00:00Z", "last_active_at": "2026-10-25T00:00:00Z"})
    };
    let split = |id: &str, first: &str, second: &str| {
        json!({"split": {"id": id, "axis": "row", "ratio": 0.5,
               "first": {"pane": first}, "second": {"pane": second}}})
    };
    serde_json::from_value(json!({
        "rev": 7, "next_id": 15,
        "sessions": [
            session("s1", "work", active_tab, json!([
                {"id": "t2", "name": null, "focused": "p5", "zoomed": null,
                 "layout": split("x4", "p3", "p5")},
                {"id": "t6", "name": "logs", "focused": "p7", "zoomed": null,
                 "layout": {"pane": "p7"}},
                {"id": "t8", "name": null, "focused": "p11", "zoomed": "p11",
                 "layout": split("x10", "p9", "p11")},
            ])),
            session("s12", "other", "t13", json!([
                {"id": "t13", "name": null, "focused": "p14", "zoomed": null,
                 "layout": {"pane": "p14"}},
            ])),
        ],
        "panes": serde_json::Map::from_iter([
            pane(3, "code", false), pane(5, "nvim", false), pane(7, "tail", true),
            pane(9, "~", false), pane(11, "htop", false), pane(14, "~", false),
        ]),
    }))
    .unwrap()
}

fn rect(x: u32, y: u32, width: u32, height: u32) -> Rect {
    Rect {
        x,
        y,
        width,
        height,
    }
}

fn session(id: u64) -> SessionId {
    format!("s{id}").parse().unwrap()
}

fn chrome_of(active_tab: &str, width: u32, height: u32) -> Chrome {
    chrome(
        &workspace(active_tab),
        session(1),
        &palette(),
        &window(),
        width,
        height,
    )
    .unwrap()
}

#[test]
fn the_top_bar_shows_the_session_and_its_tabs() {
    let chrome = chrome_of("t2", 800, 600);
    assert_eq!(chrome.title, "work — Shoal");
    assert_eq!(
        chrome.bar,
        Color {
            rgb: TOP_BAR,
            alpha: 0.88
        }
    );
    assert_eq!(chrome.chip.name, "work");
    assert_eq!(chrome.chip.text, Color::opaque(FOREGROUND));

    let titles: Vec<_> = chrome.tabs.iter().map(|tab| tab.title.as_str()).collect();
    // A tab shows its focused pane's title, or the name the user gave it.
    assert_eq!(titles, ["nvim", "logs", "htop"]);
    let [active, ringing, zoomed] = &chrome.tabs[..] else {
        panic!("three tabs")
    };
    assert_eq!(active.text, Color::opaque(FOREGROUND));
    assert_eq!(active.underline, Some(Color::opaque(ACCENT)));
    assert_eq!(active.bell, None);
    assert_eq!(active.glyph, Glyph::Terminal);
    assert_eq!(ringing.text, Color::opaque(INACTIVE));
    assert_eq!(ringing.underline, None);
    assert_eq!(ringing.bell, Some(Color::opaque(ACCENT)));
    assert_eq!(zoomed.glyph, Glyph::Zoomed);
}

#[test]
fn split_panes_fill_the_padded_area_and_the_focused_one_is_marked() {
    // 800 × 600 less 8 px padding is 784 × 584 at (8, 8). The 6 px gap leaves
    // 778 px to split evenly: 389 each, the second starting at 8 + 389 + 6.
    let chrome = chrome_of("t2", 800, 600);
    let fill = Color {
        rgb: BACKGROUND,
        alpha: 0.88,
    };
    assert_eq!(
        chrome.panes,
        [
            PaneBox {
                pane: "p3".parse().unwrap(),
                rect: rect(8, 8, 389, 584),
                radius: 10.0,
                fill,
                border: Color {
                    rgb: LIGHTER,
                    alpha: 0.6
                },
                border_width: 1.0,
            },
            PaneBox {
                pane: "p5".parse().unwrap(),
                rect: rect(403, 8, 389, 584),
                radius: 10.0,
                fill,
                border: Color::opaque(ACCENT),
                border_width: 1.5,
            },
        ]
    );
    // A point finds its box; the gap and the padding find none.
    let at = |x, y| pane_at(&chrome.panes, (x, y)).map(|pane| pane.to_string());
    assert_eq!(at(8.0, 8.0).as_deref(), Some("p3"));
    assert_eq!(at(396.9, 591.9).as_deref(), Some("p3"));
    assert_eq!(at(403.0, 300.0).as_deref(), Some("p5"));
    assert_eq!(at(400.0, 300.0), None);
    assert_eq!(at(4.0, 300.0), None);
    assert_eq!(at(300.0, 592.0), None);
    // The gap between them, x 397..403, drags the split.
    assert_eq!(
        chrome.dividers,
        [Divider {
            split: "x4".parse().unwrap(),
            axis: Axis::Row,
            ratio: 0.5,
            area: rect(8, 8, 784, 584),
            gap: rect(397, 8, 6, 584),
        }]
    );
}

#[test]
fn one_visible_pane_has_no_focus_border() {
    let unfocused = (
        Color {
            rgb: LIGHTER,
            alpha: 0.6,
        },
        1.0,
    );
    let single = chrome_of("t6", 800, 600);
    let borders: Vec<_> = single
        .panes
        .iter()
        .map(|pane| (pane.border, pane.border_width))
        .collect();
    assert_eq!(borders, [unfocused]);
    assert_eq!(single.panes[0].rect, rect(8, 8, 784, 584));

    // A zoomed pane fills the area alone; the other keeps running unseen.
    let zoomed = chrome_of("t8", 800, 600);
    let shown: Vec<_> = zoomed
        .panes
        .iter()
        .map(|pane| (pane.pane.to_string(), pane.rect, pane.border_width))
        .collect();
    assert_eq!(shown, [("p11".to_owned(), rect(8, 8, 784, 584), 1.0)]);
    // Nothing to drag while zoomed, or with one pane.
    assert!(zoomed.dividers.is_empty());
    assert!(single.dividers.is_empty());
}

#[test]
fn window_settings_change_the_boxes() {
    let config = WindowConfig {
        opacity: 0.5,
        padding: 0,
        gap: 10,
        radius: 4,
    };
    let chrome = chrome(&workspace("t2"), session(1), &palette(), &config, 210, 100).unwrap();
    let boxes: Vec<_> = chrome
        .panes
        .iter()
        .map(|pane| (pane.rect, pane.radius, pane.fill.alpha))
        .collect();
    assert_eq!(
        boxes,
        [
            (rect(0, 0, 100, 100), 4.0, 0.5),
            (rect(110, 0, 100, 100), 4.0, 0.5)
        ]
    );
    assert_eq!(chrome.bar.alpha, 0.5);
}

#[test]
fn a_window_smaller_than_its_padding_draws_empty_boxes() {
    let chrome = chrome_of("t2", 10, 0);
    for pane in &chrome.panes {
        assert_eq!((pane.rect.width, pane.rect.height), (0, 0), "{pane:?}");
    }
}

#[test]
fn another_session_and_a_closed_one() {
    let workspace = workspace("t2");
    let other = chrome(&workspace, session(12), &palette(), &window(), 800, 600).unwrap();
    assert_eq!(other.title, "other — Shoal");
    assert_eq!(other.tabs.len(), 1);
    assert_eq!(
        chrome(&workspace, session(99), &palette(), &window(), 800, 600),
        None
    );
}
