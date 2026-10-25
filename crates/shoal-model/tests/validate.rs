mod common;

use shoal_model::{
    AnyId, Axis, PaneId, PaneStatus, SessionId, SplitId, TabId, ValidationError, Workspace,
    validate,
};

use common::{leaf, pane, split, workspace};

fn rejects(workspace: &Workspace, expected: ValidationError) {
    assert_eq!(validate(workspace), Err(expected));
}

#[test]
fn accepts_fixture() {
    assert_eq!(validate(&workspace()), Ok(()));
}

#[test]
fn accepts_empty_workspace() {
    let empty = Workspace {
        rev: 0,
        next_id: 1,
        sessions: Vec::new(),
        panes: Default::default(),
    };
    assert_eq!(validate(&empty), Ok(()));
}

#[test]
fn rejects_pane_stored_under_another_key() {
    let mut ws = workspace();
    ws.panes.get_mut(&PaneId::new(7)).expect("p7").id = PaneId::new(4);
    rejects(
        &ws,
        ValidationError::PaneKeyMismatch {
            key: PaneId::new(7),
            id: PaneId::new(4),
        },
    );
}

#[test]
fn rejects_session_without_tabs() {
    let mut ws = workspace();
    let removed = ws.sessions[1].tabs.remove(0);
    ws.panes.remove(&removed.focused);
    rejects(&ws, ValidationError::EmptySession(SessionId::new(10)));
}

#[test]
fn rejects_active_tab_from_another_session() {
    let mut ws = workspace();
    ws.sessions[1].active_tab = TabId::new(2);
    rejects(
        &ws,
        ValidationError::ActiveTabOutside {
            session: SessionId::new(10),
            tab: TabId::new(2),
        },
    );
}

#[test]
fn rejects_focus_on_pane_in_another_tab() {
    let mut ws = workspace();
    ws.sessions[0].tabs[1].focused = PaneId::new(4);
    rejects(
        &ws,
        ValidationError::FocusedOutside {
            tab: TabId::new(8),
            pane: PaneId::new(4),
        },
    );
}

#[test]
fn rejects_zoom_on_pane_in_another_tab() {
    let mut ws = workspace();
    ws.sessions[1].tabs[0].zoomed = Some(PaneId::new(9));
    rejects(
        &ws,
        ValidationError::ZoomedOutside {
            tab: TabId::new(11),
            pane: PaneId::new(9),
        },
    );
}

#[test]
fn rejects_pane_in_two_tabs() {
    let mut ws = workspace();
    ws.sessions[1].tabs[0].layout = split(13, Axis::Row, 0.5, leaf(12), leaf(4));
    rejects(&ws, ValidationError::PaneInManyPlaces(PaneId::new(4)));
}

#[test]
fn rejects_pane_twice_in_one_layout() {
    let mut ws = workspace();
    ws.sessions[0].tabs[1].layout = split(13, Axis::Column, 0.5, leaf(9), leaf(9));
    rejects(&ws, ValidationError::PaneInManyPlaces(PaneId::new(9)));
}

#[test]
fn rejects_layout_pane_missing_from_panes() {
    let mut ws = workspace();
    ws.panes.remove(&PaneId::new(7));
    rejects(&ws, ValidationError::UnknownPane(PaneId::new(7)));
}

#[test]
fn rejects_pane_in_no_layout() {
    let mut ws = workspace();
    let orphan = pane(14, None, "~", PaneStatus::Running);
    ws.panes.insert(orphan.id, orphan);
    rejects(&ws, ValidationError::PaneNotInLayout(PaneId::new(14)));
}

#[test]
fn rejects_repeated_id_of_one_kind() {
    let mut ws = workspace();
    let nested = split(3, Axis::Column, 0.5, leaf(6), leaf(7));
    ws.sessions[0].tabs[0].layout = split(3, Axis::Row, 0.6, leaf(4), nested);
    rejects(
        &ws,
        ValidationError::DuplicateId {
            first: AnyId::Split(SplitId::new(3)),
            second: AnyId::Split(SplitId::new(3)),
        },
    );
}

#[test]
fn rejects_number_shared_across_kinds() {
    let mut ws = workspace();
    ws.sessions[1].tabs[0].id = TabId::new(7);
    ws.sessions[1].active_tab = TabId::new(7);
    rejects(
        &ws,
        ValidationError::DuplicateId {
            first: AnyId::Tab(TabId::new(7)),
            second: AnyId::Pane(PaneId::new(7)),
        },
    );
}

#[test]
fn rejects_id_not_below_next_id() {
    let mut ws = workspace();
    ws.next_id = 12;
    rejects(
        &ws,
        ValidationError::IdNotBelowNextId {
            id: AnyId::Pane(PaneId::new(12)),
            next_id: 12,
        },
    );
}

#[test]
fn rejects_repeated_session_name() {
    let mut ws = workspace();
    ws.sessions[1].name = "main".into();
    rejects(
        &ws,
        ValidationError::DuplicateSessionName {
            first: SessionId::new(1),
            second: SessionId::new(10),
            name: "main".into(),
        },
    );
}

fn with_ratios(outer: f32, inner: f32) -> Workspace {
    let mut ws = workspace();
    let nested = split(5, Axis::Column, inner, leaf(6), leaf(7));
    ws.sessions[0].tabs[0].layout = split(3, Axis::Row, outer, leaf(4), nested);
    ws
}

#[test]
fn accepts_ratios_at_the_clamp_bounds() {
    assert_eq!(validate(&with_ratios(0.1, 0.9)), Ok(()));
}

#[test]
fn rejects_ratio_outside_clamp() {
    rejects(
        &with_ratios(0.6, 0.95),
        ValidationError::RatioOutOfRange {
            split: SplitId::new(5),
            ratio: 0.95,
        },
    );
}

#[test]
fn rejects_nan_ratio() {
    let result = validate(&with_ratios(f32::NAN, 0.5));
    assert!(
        matches!(
            result,
            Err(ValidationError::RatioOutOfRange { split, ratio })
                if split == SplitId::new(3) && ratio.is_nan()
        ),
        "{result:?}"
    );
}
