//! Command targets (M1 spec §11): an id (`s3`, `t12`, `p40`), a session
//! name, or `.` for the pane the command runs in. Pure: callers pass the
//! workspace and `$SHOAL_PANE` in.

use shoal_client::model::{PaneId, SessionId, TabId, Workspace};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Session(SessionId),
    Tab(TabId),
    Pane(PaneId),
}

/// Find `name` in `workspace`. Ids win over session names.
pub fn resolve(workspace: &Workspace, name: &str, here: Option<&str>) -> Result<Target, String> {
    if name == "." {
        let here = here.ok_or("`.` means this pane, but SHOAL_PANE isn't set")?;
        let pane: PaneId = here
            .parse()
            .map_err(|_| format!("SHOAL_PANE is {here:?}, not a pane id"))?;
        return if workspace.panes.contains_key(&pane) {
            Ok(Target::Pane(pane))
        } else {
            Err(format!("this pane ({pane}) is gone"))
        };
    }
    if let Ok(pane) = name.parse::<PaneId>() {
        if workspace.panes.contains_key(&pane) {
            return Ok(Target::Pane(pane));
        }
    } else if let Ok(tab) = name.parse::<TabId>() {
        if workspace
            .sessions
            .iter()
            .any(|session| session.tabs.iter().any(|item| item.id == tab))
        {
            return Ok(Target::Tab(tab));
        }
    } else if let Ok(session) = name.parse::<SessionId>()
        && workspace.sessions.iter().any(|item| item.id == session)
    {
        return Ok(Target::Session(session));
    }
    workspace
        .sessions
        .iter()
        .find(|session| session.name == name)
        .map(|session| Target::Session(session.id))
        .ok_or_else(|| format!("no session, tab or pane called {name:?}"))
}

/// The pane a target means for `send` and `capture`: a session's active tab's
/// focused pane, or a tab's focused pane.
pub fn pane(workspace: &Workspace, target: Target) -> PaneId {
    let tab_focus = |tab: TabId| {
        workspace
            .sessions
            .iter()
            .flat_map(|session| &session.tabs)
            .find(|item| item.id == tab)
            .map(|item| item.focused)
            .expect("invariant: a resolved tab exists")
    };
    match target {
        Target::Pane(pane) => pane,
        Target::Tab(tab) => tab_focus(tab),
        Target::Session(session) => {
            let session = workspace
                .sessions
                .iter()
                .find(|item| item.id == session)
                .expect("invariant: a resolved session exists");
            tab_focus(session.active_tab)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Session `main` (s1) with tabs t2 (p3, p5) and t6 (p7); session `p3` (s8)
    /// with tab t9 (p10), named like a pane id.
    fn workspace() -> Workspace {
        serde_json::from_value(serde_json::json!({
            "rev": 1, "next_id": 11,
            "sessions": [
                {"id": "s1", "name": "main", "active_tab": "t6", "created_at": "2026-10-25T00:00:00Z",
                 "last_active_at": "2026-10-25T00:00:00Z", "windows": 0, "tabs": [
                    {"id": "t2", "name": null, "focused": "p5", "zoomed": null,
                     "layout": {"split": {"id": "x4", "axis": "row", "ratio": 0.5,
                                "first": {"pane": "p3"}, "second": {"pane": "p5"}}}},
                    {"id": "t6", "name": null, "focused": "p7", "zoomed": null, "layout": {"pane": "p7"}}]},
                {"id": "s8", "name": "p3", "active_tab": "t9", "created_at": "2026-10-25T00:00:00Z",
                 "last_active_at": "2026-10-25T00:00:00Z", "windows": 0, "tabs": [
                    {"id": "t9", "name": null, "focused": "p10", "zoomed": null, "layout": {"pane": "p10"}}]}],
            "panes": Object::from_iter([3, 5, 7, 10].map(|id| (format!("p{id}"), serde_json::json!({
                "id": format!("p{id}"), "kind": {"kind": "shell", "argv": []}, "size": {"cols": 80, "rows": 24},
                "cwd": null, "program": null, "title": "", "status": {"state": "running"}, "bell": false,
                "restored_from": null}))))
        }))
        .unwrap()
    }

    type Object = serde_json::Map<String, serde_json::Value>;

    #[test]
    fn ids_names_and_dot_resolve() {
        let workspace = workspace();
        let ok = |name, here| resolve(&workspace, name, here).unwrap();
        assert_eq!(ok("s1", None), Target::Session(SessionId::new(1)));
        assert_eq!(ok("t6", None), Target::Tab(TabId::new(6)));
        assert_eq!(ok("p5", None), Target::Pane(PaneId::new(5)));
        assert_eq!(ok("main", None), Target::Session(SessionId::new(1)));
        // An existing id wins over a session with that name.
        assert_eq!(ok("p3", None), Target::Pane(PaneId::new(3)));
        assert_eq!(ok(".", Some("p7")), Target::Pane(PaneId::new(7)));
    }

    #[test]
    fn unknown_targets_and_a_missing_pane_variable_are_errors() {
        let workspace = workspace();
        let error = |name, here| resolve(&workspace, name, here).unwrap_err();
        assert_eq!(
            error("nope", None),
            "no session, tab or pane called \"nope\""
        );
        assert_eq!(error("p99", None), "no session, tab or pane called \"p99\"");
        assert_eq!(error("t3", None), "no session, tab or pane called \"t3\"");
        assert_eq!(
            error(".", None),
            "`.` means this pane, but SHOAL_PANE isn't set"
        );
        assert_eq!(
            error(".", Some("main")),
            "SHOAL_PANE is \"main\", not a pane id"
        );
        assert_eq!(error(".", Some("p99")), "this pane (p99) is gone");
    }

    #[test]
    fn sessions_and_tabs_mean_their_focused_pane() {
        let workspace = workspace();
        assert_eq!(
            pane(&workspace, Target::Session(SessionId::new(1))),
            PaneId::new(7)
        );
        assert_eq!(pane(&workspace, Target::Tab(TabId::new(2))), PaneId::new(5));
        assert_eq!(
            pane(&workspace, Target::Pane(PaneId::new(3))),
            PaneId::new(3)
        );
    }
}
