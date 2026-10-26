//! Which panes a window keeps attached (M1 spec §7.8): every pane of the 8
//! most recently viewed tabs, so switching back to one is instant. Other
//! tabs are detached and cost the window nothing.

use std::collections::BTreeSet;

use shoal_model::{PaneId, Tab, TabId, Workspace};

/// How many recently viewed tabs stay attached.
pub const KEPT_TABS: usize = 8;

/// Recently viewed tabs, newest first.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Recent(Vec<TabId>);

impl Recent {
    /// `active` is on screen now; tabs that no longer exist are forgotten.
    pub fn view(&mut self, workspace: &Workspace, active: TabId) {
        self.0
            .retain(|tab| *tab != active && find(workspace, *tab).is_some());
        self.0.insert(0, active);
        self.0.truncate(KEPT_TABS);
    }

    /// Every pane of the kept tabs.
    pub fn panes(&self, workspace: &Workspace) -> BTreeSet<PaneId> {
        let mut panes = BTreeSet::new();
        for tab in self.0.iter().filter_map(|tab| find(workspace, *tab)) {
            tab.layout.for_each_pane(&mut |pane| {
                panes.insert(pane);
            });
        }
        panes
    }
}

fn find(workspace: &Workspace, id: TabId) -> Option<&Tab> {
    workspace
        .sessions
        .iter()
        .flat_map(|session| &session.tabs)
        .find(|tab| tab.id == id)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// One session with tabs t1..=t10; tab `tN` holds pane `pN`, and t1 is
    /// split with p100 too.
    fn workspace(tabs: impl IntoIterator<Item = u64>) -> Workspace {
        let tabs: Vec<_> = tabs
            .into_iter()
            .map(|n| {
                let layout = if n == 1 {
                    json!({"split": {"id": "x99", "axis": "row", "ratio": 0.5,
                           "first": {"pane": "p1"}, "second": {"pane": "p100"}}})
                } else {
                    json!({"pane": format!("p{n}")})
                };
                json!({"id": format!("t{n}"), "name": null, "focused": format!("p{n}"),
                       "zoomed": null, "layout": layout})
            })
            .collect();
        serde_json::from_value(json!({
            "rev": 1, "next_id": 200,
            "sessions": [{"id": "s50", "name": "main", "active_tab": "t1", "tabs": tabs,
                          "windows": 1, "created_at": "2026-10-25T00:00:00Z",
                          "last_active_at": "2026-10-25T00:00:00Z"}],
            "panes": {},
        }))
        .unwrap()
    }

    fn tab(n: u64) -> TabId {
        format!("t{n}").parse().unwrap()
    }

    fn panes(ns: &[u64]) -> BTreeSet<PaneId> {
        ns.iter()
            .map(|n| format!("p{n}").parse().unwrap())
            .collect()
    }

    #[test]
    fn the_eight_most_recently_viewed_tabs_stay_attached() {
        let all = workspace(1..=10);
        let mut recent = Recent::default();
        recent.view(&all, tab(1));
        assert_eq!(recent.panes(&all), panes(&[1, 100]), "a split tab's panes");
        for n in 2..=9 {
            recent.view(&all, tab(n));
        }
        // t1 was viewed nine tabs ago, so it is dropped.
        assert_eq!(recent.panes(&all), panes(&[2, 3, 4, 5, 6, 7, 8, 9]));
        // Viewing t2 again makes it newest; t10 then pushes out t3.
        recent.view(&all, tab(2));
        recent.view(&all, tab(10));
        assert_eq!(recent.panes(&all), panes(&[2, 4, 5, 6, 7, 8, 9, 10]));
    }

    #[test]
    fn closed_tabs_are_forgotten() {
        let mut recent = Recent::default();
        let all = workspace(1..=3);
        for n in [1, 2, 3] {
            recent.view(&all, tab(n));
        }
        let fewer = workspace([1, 3]);
        assert_eq!(recent.panes(&fewer), panes(&[1, 3, 100]));
        recent.view(&fewer, tab(3));
        assert_eq!(recent, Recent(vec![tab(3), tab(1)]));
    }
}
