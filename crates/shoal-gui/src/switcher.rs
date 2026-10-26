//! Session switcher state and ranking (§7.6). No GPUI or daemon calls.
use crate::field::TextField;
use shoal_model::{Session, SessionId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Session(SessionId),
    New,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub target: Target,
    pub name: String,
    pub tabs: usize,
    pub on_screen: bool,
}

pub enum Mode {
    Browse,
    Rename {
        session: SessionId,
        field: TextField,
    },
    Confirm {
        session: SessionId,
        name: String,
    },
}

pub struct Switcher {
    pub query: TextField,
    pub selected: Target,
    pub mode: Mode,
    pub busy: bool,
    pub error: Option<String>,
}

impl Switcher {
    pub fn new(current: SessionId) -> Self {
        Self {
            query: TextField::new(""),
            selected: Target::Session(current),
            mode: Mode::Browse,
            busy: false,
            error: None,
        }
    }

    pub fn field(&self) -> Option<&TextField> {
        if self.busy {
            return None;
        }
        match &self.mode {
            Mode::Browse => Some(&self.query),
            Mode::Rename { field, .. } => Some(field),
            Mode::Confirm { .. } => None,
        }
    }

    pub fn field_mut(&mut self) -> Option<&mut TextField> {
        if self.busy {
            return None;
        }
        match &mut self.mode {
            Mode::Browse => Some(&mut self.query),
            Mode::Rename { field, .. } => Some(field),
            Mode::Confirm { .. } => None,
        }
    }

    pub fn rows(&self, sessions: &[Session], current: SessionId) -> Vec<Row> {
        rows(sessions, current, self.query.text())
    }

    pub fn filtered(&mut self, sessions: &[Session], current: SessionId) {
        if matches!(self.mode, Mode::Browse) {
            self.selected = self.rows(sessions, current)[0].target;
            self.error = None;
        }
    }

    pub fn refresh(&mut self, sessions: &[Session], current: SessionId) {
        let rows = self.rows(sessions, current);
        if !rows.iter().any(|row| row.target == self.selected) {
            self.selected = rows[0].target;
        }
        let editing = match self.mode {
            Mode::Rename { session, .. } | Mode::Confirm { session, .. } => Some(session),
            Mode::Browse => None,
        };
        if editing.is_some_and(|id| !sessions.iter().any(|s| s.id == id)) {
            self.mode = Mode::Browse;
            self.error = None;
        }
    }

    pub fn step(&mut self, sessions: &[Session], current: SessionId, forward: bool) -> usize {
        let rows = self.rows(sessions, current);
        let at = rows
            .iter()
            .position(|r| r.target == self.selected)
            .unwrap_or(0);
        let at = if forward {
            (at + 1) % rows.len()
        } else {
            (at + rows.len() - 1) % rows.len()
        };
        self.selected = rows[at].target;
        at
    }
}

pub fn rows(sessions: &[Session], current: SessionId, query: &str) -> Vec<Row> {
    let query = query.trim().to_lowercase();
    let mut matches: Vec<_> = sessions
        .iter()
        .filter_map(|session| {
            score(&session.name.to_lowercase(), &query).map(|score| (score, session))
        })
        .collect();
    matches.sort_by_key(|(score, session)| (*score, session.created_at, session.id));
    let mut rows: Vec<_> = matches
        .into_iter()
        .map(|(_, session)| Row {
            target: Target::Session(session.id),
            name: session.name.clone(),
            tabs: session.tabs.len(),
            on_screen: session.windows > u32::from(session.id == current),
        })
        .collect();
    rows.push(Row {
        target: Target::New,
        name: "New session".into(),
        tabs: 0,
        on_screen: false,
    });
    rows
}

/// Exact, prefix, contiguous, then subsequence with the fewest skipped letters.
fn score(name: &str, query: &str) -> Option<(u8, usize)> {
    if query.is_empty() {
        return Some((0, 0));
    }
    if name == query {
        return Some((0, 0));
    }
    if name.starts_with(query) {
        return Some((1, 0));
    }
    if name.contains(query) {
        return Some((2, 0));
    }
    let needle: Vec<_> = query.chars().collect();
    let mut starts = vec![None; needle.len()];
    let mut gaps = None;
    for (at, ch) in name.chars().enumerate() {
        for i in (0..needle.len()).rev() {
            if ch != needle[i] {
                continue;
            }
            let start = if i == 0 { Some(at) } else { starts[i - 1] };
            if let Some(start) = start {
                starts[i] = Some(start);
                if i + 1 == needle.len() {
                    let skipped = at + 1 - start - needle.len();
                    gaps = Some(gaps.map_or(skipped, |best: usize| best.min(skipped)));
                }
            }
        }
    }
    gaps.map(|gaps| (3, gaps))
}

/// Session navigation follows creation order, regardless of names or activity.
pub fn adjacent(sessions: &[Session], current: SessionId, forward: bool) -> Option<SessionId> {
    let mut ordered: Vec<_> = sessions.iter().collect();
    ordered.sort_by_key(|s| (s.created_at, s.id));
    let at = ordered.iter().position(|s| s.id == current)?;
    let next = if forward {
        (at + 1) % ordered.len()
    } else {
        (at + ordered.len() - 1) % ordered.len()
    };
    Some(ordered[next].id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shoal_model::{Layout, PaneId, Tab, TabId};
    fn sessions(names: &[&str]) -> Vec<Session> {
        names
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let n = i as u64 + 1;
                Session {
                    id: SessionId::new(n),
                    name: (*name).into(),
                    windows: 0,
                    created_at: "2026-10-26T00:00:00Z".parse().unwrap(),
                    last_active_at: "2026-10-26T00:00:00Z".parse().unwrap(),
                    active_tab: TabId::new(n),
                    tabs: vec![Tab {
                        id: TabId::new(n),
                        name: None,
                        focused: PaneId::new(n),
                        zoomed: None,
                        layout: Layout::Pane(PaneId::new(n)),
                    }],
                }
            })
            .collect()
    }
    #[test]
    fn fuzzy_ranking_prefers_exact_prefix_contiguous_then_compact_subsequences() {
        let all = sessions(&[
            "a---b---c",
            "xabc",
            "ABCtools",
            "abc",
            "a-b-c",
            "nomatch",
            "ABC-tests",
        ]);
        let ranked: Vec<_> = rows(&all, all[0].id, " AbC ")
            .into_iter()
            .map(|r| r.name)
            .collect();
        assert_eq!(
            ranked,
            [
                "abc",
                "ABCtools",
                "ABC-tests",
                "xabc",
                "a-b-c",
                "a---b---c",
                "New session"
            ]
        );
        assert_eq!(
            score("axabxc", "abc"),
            Some((3, 1)),
            "use the tighter later match"
        );
        assert_eq!(score("ab", "abb"), None, "one letter cannot match twice");
        assert_eq!(score("日本語の仕事", "日仕"), Some((3, 3)));
        assert_eq!(rows(&all, all[0].id, "missing")[0].target, Target::New);
    }
    #[test]
    fn window_markers_exclude_this_window_and_counts_follow_the_session() {
        let mut all = sessions(&["main", "work", "logs"]);
        all[0].windows = 1;
        all[1].windows = 1;
        all[1].tabs.push(Tab {
            id: TabId::new(100),
            name: None,
            focused: PaneId::new(100),
            zoomed: None,
            layout: Layout::Pane(PaneId::new(100)),
        });
        let rows = rows(&all, all[0].id, "");
        assert_eq!(
            rows.iter()
                .map(|r| (r.tabs, r.on_screen))
                .collect::<Vec<_>>(),
            [(1, false), (2, true), (1, false), (0, false)]
        );
        all[0].windows = 2;
        assert!(super::rows(&all, all[0].id, "")[0].on_screen);
    }
    #[test]
    fn selection_survives_updates_and_a_closed_rename_target_is_cancelled() {
        let mut all = sessions(&["main", "work"]);
        let current = all[0].id;
        let mut picker = Switcher::new(current);
        assert_eq!(picker.step(&all, current, true), 1);
        picker.refresh(&all, current);
        assert_eq!(picker.selected, Target::Session(all[1].id));
        picker.mode = Mode::Rename {
            session: all[1].id,
            field: TextField::new("work"),
        };
        all.pop();
        picker.refresh(&all, current);
        assert!(matches!(picker.mode, Mode::Browse));
        assert_eq!(picker.selected, Target::Session(current));
        picker.query.replace(None, "absent", None, false);
        picker.filtered(&all, current);
        assert_eq!(picker.selected, Target::New);
        assert_eq!(picker.step(&all, current, false), 0);
    }
    #[test]
    fn next_and_previous_wrap_in_creation_order_after_renaming() {
        let mut all = sessions(&["z", "a", "m"]);
        let current = all[0].id;
        // Creation time beats both id and list position; names do not sort it.
        all[1].created_at = "2026-10-25T00:00:00Z".parse().unwrap();
        all.reverse();
        assert_eq!(adjacent(&all, current, true), Some(SessionId::new(3)));
        assert_eq!(adjacent(&all, current, false), Some(SessionId::new(2)));
        assert_eq!(
            adjacent(&all, SessionId::new(3), true),
            Some(SessionId::new(2))
        );
        assert_eq!(adjacent(&[], current, true), None);
        assert_eq!(adjacent(&all[..1], all[0].id, true), Some(all[0].id));
    }
}
