mod common;

use std::collections::BTreeMap;

use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use shoal_model::{
    Axis, Direction, Layout, LayoutError, PaneId, PaneStatus, Rect, Session, SessionId,
    SplitDirection, SplitId, Tab, TabId, Workspace, close, neighbor, rects, resize, set_ratio,
    split, toggle_zoom, validate,
};

#[derive(Debug, Clone)]
struct Step {
    kind: u8,
    tab: u16,
    target: u16,
    direction: Direction,
    value: f32,
    missing: bool,
}

fn steps() -> impl Strategy<Value = Step> {
    (
        0u8..7,
        any::<u16>(),
        any::<u16>(),
        prop::sample::select(vec![
            Direction::Left,
            Direction::Right,
            Direction::Up,
            Direction::Down,
        ]),
        prop_oneof![-1.0f32..2.0, any::<f32>()],
        any::<bool>(),
    )
        .prop_map(|(kind, tab, target, direction, value, missing)| Step {
            kind,
            tab,
            target,
            direction,
            value,
            missing,
        })
}

/// The real caller's bookkeeping, kept outside the layout operations. Restart
/// after an empty workspace so long sequences exercise edits beyond close cascades.
fn ensure_tab(ws: &mut Workspace) {
    if !ws.sessions.is_empty() {
        return;
    }
    let id = ws.next_id;
    let pane = common::pane(id + 2, None, "~", PaneStatus::Running);
    ws.panes.insert(pane.id, pane);
    let time = common::at("2026-10-25T12:00:00Z");
    ws.sessions.push(Session {
        id: SessionId::new(id),
        name: "main".into(),
        tabs: vec![Tab {
            id: TabId::new(id + 1),
            name: None,
            layout: Layout::Pane(PaneId::new(id + 2)),
            focused: PaneId::new(id + 2),
            zoomed: None,
        }],
        active_tab: TabId::new(id + 1),
        created_at: time,
        last_active_at: time,
        windows: 0,
    });
    ws.next_id += 3;
}

fn apply(ws: &mut Workspace, step: &Step, area: Rect, gap: u32) -> Result<(), LayoutError> {
    let tabs: Vec<_> = ws
        .sessions
        .iter()
        .enumerate()
        .flat_map(|(s, session)| (0..session.tabs.len()).map(move |t| (s, t)))
        .collect();
    let (s, t) = tabs[usize::from(step.tab) % tabs.len()];
    let tab = &mut ws.sessions[s].tabs[t];
    let mut panes = Vec::new();
    tab.layout.for_each_pane(&mut |id| panes.push(id));
    let pane = if step.missing {
        PaneId::new(u64::MAX)
    } else {
        panes[usize::from(step.target) % panes.len()]
    };
    match step.kind {
        0 | 1 => {
            let id = ws.next_id;
            split(
                tab,
                pane,
                if step.kind == 0 {
                    SplitDirection::Right
                } else {
                    SplitDirection::Down
                },
                PaneId::new(id),
                SplitId::new(id + 1),
            )?;
            let new = common::pane(id, None, "~", PaneStatus::Running);
            ws.panes.insert(new.id, new);
            ws.next_id += 2;
        }
        2 => {
            let focus = close(tab, pane)?;
            ws.panes.remove(&pane);
            if focus.is_none() {
                let session = &mut ws.sessions[s];
                session.tabs.remove(t);
                if session.tabs.is_empty() {
                    ws.sessions.remove(s);
                } else if !session.tabs.iter().any(|tab| tab.id == session.active_tab) {
                    session.active_tab = session.tabs[0].id;
                }
            }
        }
        3 => {
            resize(&mut tab.layout, pane, step.direction, step.value)?;
        }
        4 => {
            let mut splits = Vec::new();
            tab.layout.for_each_split(&mut |id| splits.push(id));
            let id = if step.missing || splits.is_empty() {
                SplitId::new(u64::MAX)
            } else {
                splits[usize::from(step.target) % splits.len()]
            };
            set_ratio(&mut tab.layout, id, step.value)?;
        }
        5 => toggle_zoom(tab, pane)?,
        6 => {
            if let Some(next) = neighbor(&rects(&tab.layout, area, gap)?, pane, step.direction) {
                tab.focused = next;
            }
        }
        _ => unreachable!(),
    }
    ws.rev += 1;
    Ok(())
}

/// Infer subtree bounds and divider strips from the returned leaves. This oracle
/// does not repeat the production ratio calculation or use its geometry helpers.
fn partition(
    layout: &Layout,
    rects: &BTreeMap<PaneId, Rect>,
    gap: u32,
    pieces: &mut Vec<Rect>,
) -> Rect {
    let Layout::Split {
        axis,
        first,
        second,
        ..
    } = layout
    else {
        let Layout::Pane(id) = layout else {
            unreachable!()
        };
        let rect = rects[id];
        pieces.push(rect);
        return rect;
    };
    let a = partition(first, rects, gap, pieces);
    let b = partition(second, rects, gap, pieces);
    let parent = Rect {
        x: a.x.min(b.x),
        y: a.y.min(b.y),
        width: (a.x + a.width).max(b.x + b.width) - a.x.min(b.x),
        height: (a.y + a.height).max(b.y + b.height) - a.y.min(b.y),
    };
    let divider = match axis {
        Axis::Row => {
            assert_eq!((a.y, a.height), (b.y, b.height));
            assert!(a.x + a.width <= b.x);
            let width = b.x - (a.x + a.width);
            assert_eq!(width, gap.min(parent.width));
            Rect {
                x: a.x + a.width,
                y: a.y,
                width,
                height: a.height,
            }
        }
        Axis::Column => {
            assert_eq!((a.x, a.width), (b.x, b.width));
            assert!(a.y + a.height <= b.y);
            let height = b.y - (a.y + a.height);
            assert_eq!(height, gap.min(parent.height));
            Rect {
                x: a.x,
                y: a.y + a.height,
                width: a.width,
                height,
            }
        }
    };
    pieces.push(divider);
    parent
}

fn assert_tiling(layout: &Layout, area: Rect, gap: u32) {
    let actual = rects(layout, area, gap).unwrap();
    let mut leaves = Vec::new();
    layout.for_each_pane(&mut |id| leaves.push(id));
    leaves.sort();
    assert_eq!(actual.keys().copied().collect::<Vec<_>>(), leaves);
    let mut pieces = Vec::new();
    assert_eq!(partition(layout, &actual, gap, &mut pieces), area);
    let mut covered = vec![false; (area.width * area.height) as usize];
    for piece in pieces {
        assert!(piece.x >= area.x && piece.y >= area.y);
        assert!(piece.x + piece.width <= area.x + area.width);
        assert!(piece.y + piece.height <= area.y + area.height);
        for y in piece.y..piece.y + piece.height {
            for x in piece.x..piece.x + piece.width {
                let index = ((y - area.y) * area.width + x - area.x) as usize;
                assert!(!covered[index], "overlap at ({x}, {y})");
                covered[index] = true;
            }
        }
    }
    assert!(covered.iter().all(|pixel| *pixel), "uncovered pixels");
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(concat!(
            env!("CARGO_MANIFEST_DIR"), "/tests/layout_sequences.proptest-regressions"
        )))),
        ..ProptestConfig::default()
    })]

    #[test]
    fn random_layout_operations_preserve_all_invariants_and_tile_exactly(
        operations in prop::collection::vec(steps(), 1..100),
        x in 0u32..10, y in 0u32..10,
        width in 0u32..32, height in 0u32..24,
        gap in 0u32..40,
    ) {
        let mut ws = common::workspace();
        let area = Rect { x, y, width, height };
        for operation in operations {
            ensure_tab(&mut ws);
            let before = ws.clone();
            if apply(&mut ws, &operation, area, gap).is_err() {
                prop_assert_eq!(&ws, &before, "rejected operation mutated workspace: {:?}", operation);
            }
            prop_assert_eq!(validate(&ws), Ok(()), "after {:?}", operation);
            for tab in ws.sessions.iter().flat_map(|session| &session.tabs) {
                assert_tiling(&tab.layout, area, gap);
            }
        }
    }
}
