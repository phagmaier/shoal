//! Existing §10 workspace actions as protocol calls. Later-task actions
//! remain reserved by the keymap and never leak their chords to a program.

use shoal_client::proto::Call;
use shoal_config::Action;
use shoal_model::{Direction, Rect, Session, SplitDirection, neighbor, rects};

pub fn call(action: Action, session: &Session, area: Rect, gap: u32) -> Option<Call> {
    let tabs = &session.tabs;
    let index = tabs.iter().position(|tab| tab.id == session.active_tab)?;
    let tab = &tabs[index];
    let pane = tab.focused;
    let activate = |index: usize| {
        Some(Call::TabActivate {
            tab: tabs.get(index)?.id,
        })
    };
    match action {
        Action::TabNew => Some(Call::TabCreate {
            session: session.id,
            cwd: None,
            index: None,
            argv: None,
        }),
        Action::TabClose => Some(Call::TabClose { tab: tab.id }),
        Action::TabNext => activate((index + 1) % tabs.len()),
        Action::TabPrev => activate((index + tabs.len() - 1) % tabs.len()),
        Action::TabGoto1 => activate(0),
        Action::TabGoto2 => activate(1),
        Action::TabGoto3 => activate(2),
        Action::TabGoto4 => activate(3),
        Action::TabGoto5 => activate(4),
        Action::TabGoto6 => activate(5),
        Action::TabGoto7 => activate(6),
        Action::TabGoto8 => activate(7),
        Action::TabGoto9 => activate(tabs.len() - 1),
        Action::TabMoveLeft | Action::TabMoveRight => {
            let next = if action == Action::TabMoveLeft {
                index.checked_sub(1)?
            } else {
                (index + 1).min(tabs.len() - 1)
            };
            (next != index).then_some(Call::TabMove {
                tab: tab.id,
                index: next as u64,
            })
        }
        Action::PaneSplitRight | Action::PaneSplitDown => Some(Call::PaneSplit {
            pane,
            direction: if action == Action::PaneSplitRight {
                SplitDirection::Right
            } else {
                SplitDirection::Down
            },
            cwd: None,
        }),
        Action::PaneClose => Some(Call::PaneClose { pane }),
        Action::PaneZoom => Some(Call::PaneZoom {
            pane,
            zoomed: tab.zoomed.is_none(),
        }),
        Action::PaneFocusLeft
        | Action::PaneFocusRight
        | Action::PaneFocusUp
        | Action::PaneFocusDown => {
            // Use the unzoomed layout too: focusing another pane while
            // zoomed moves zoom to it, as §4.4 requires.
            let boxes = rects(&tab.layout, area, gap).ok()?;
            Some(Call::PaneFocus {
                pane: neighbor(&boxes, pane, direction(action)?)?,
            })
        }
        Action::PaneResizeLeft
        | Action::PaneResizeRight
        | Action::PaneResizeUp
        | Action::PaneResizeDown => Some(Call::PaneResize {
            pane,
            direction: direction(action)?,
            cells: 2,
        }),
        _ => None,
    }
}

fn direction(action: Action) -> Option<Direction> {
    match action {
        Action::PaneFocusLeft | Action::PaneResizeLeft => Some(Direction::Left),
        Action::PaneFocusRight | Action::PaneResizeRight => Some(Direction::Right),
        Action::PaneFocusUp | Action::PaneResizeUp => Some(Direction::Up),
        Action::PaneFocusDown | Action::PaneResizeDown => Some(Direction::Down),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shoal_model::{Layout, PaneId, SessionId, Tab, TabId};

    fn session() -> Session {
        Session {
            id: SessionId::new(1),
            name: "test".into(),
            active_tab: TabId::new(3),
            created_at: "2026-10-26T00:00:00Z".parse().unwrap(),
            last_active_at: "2026-10-26T00:00:00Z".parse().unwrap(),
            windows: 0,
            tabs: (2..=4)
                .map(|n| Tab {
                    id: TabId::new(n),
                    name: None,
                    layout: Layout::Pane(PaneId::new(n + 10)),
                    focused: PaneId::new(n + 10),
                    zoomed: None,
                })
                .collect(),
        }
    }

    #[test]
    fn tab_actions_wrap_select_last_and_stop_moving_at_edges() {
        let mut s = session();
        let run = |action, s: &Session| {
            call(
                action,
                s,
                Rect {
                    x: 0,
                    y: 0,
                    width: 800,
                    height: 600,
                },
                8,
            )
        };
        assert_eq!(
            run(Action::TabNext, &s),
            Some(Call::TabActivate { tab: TabId::new(4) })
        );
        assert_eq!(
            run(Action::TabGoto9, &s),
            Some(Call::TabActivate { tab: TabId::new(4) })
        );
        assert_eq!(run(Action::TabGoto8, &s), None);
        assert_eq!(
            run(Action::TabMoveLeft, &s),
            Some(Call::TabMove {
                tab: TabId::new(3),
                index: 0
            })
        );
        s.active_tab = TabId::new(2);
        assert_eq!(
            run(Action::TabPrev, &s),
            Some(Call::TabActivate { tab: TabId::new(4) })
        );
        assert_eq!(run(Action::TabMoveLeft, &s), None);
        s.active_tab = TabId::new(4);
        assert_eq!(
            run(Action::TabNext, &s),
            Some(Call::TabActivate { tab: TabId::new(2) })
        );
        assert_eq!(run(Action::TabMoveRight, &s), None);
        assert_eq!(
            run(Action::PaneResizeUp, &s),
            Some(Call::PaneResize {
                pane: PaneId::new(14),
                direction: Direction::Up,
                cells: 2
            })
        );
    }

    #[test]
    fn spatial_focus_uses_the_layout_even_while_zoomed() {
        let mut s = session();
        let tab = &mut s.tabs[1];
        tab.layout = Layout::Split {
            id: shoal_model::SplitId::new(20),
            axis: shoal_model::Axis::Row,
            ratio: 0.5,
            first: Box::new(Layout::Pane(PaneId::new(13))),
            second: Box::new(Layout::Pane(PaneId::new(19))),
        };
        tab.zoomed = Some(PaneId::new(13));
        let area = Rect {
            x: 8,
            y: 8,
            width: 800,
            height: 600,
        };
        assert_eq!(
            call(Action::PaneFocusRight, &s, area, 8),
            Some(Call::PaneFocus {
                pane: PaneId::new(19)
            })
        );
        assert_eq!(call(Action::PaneFocusLeft, &s, area, 8), None);
        assert_eq!(
            call(Action::PaneZoom, &s, area, 8),
            Some(Call::PaneZoom {
                pane: PaneId::new(13),
                zoomed: false
            })
        );
        assert_eq!(
            call(Action::PaneSplitDown, &s, area, 8),
            Some(Call::PaneSplit {
                pane: PaneId::new(13),
                direction: SplitDirection::Down,
                cwd: None
            })
        );
        assert_eq!(
            call(Action::PaneClose, &s, area, 8),
            Some(Call::PaneClose {
                pane: PaneId::new(13)
            })
        );
        assert_eq!(
            call(Action::TabClose, &s, area, 8),
            Some(Call::TabClose { tab: TabId::new(3) })
        );
    }
}
