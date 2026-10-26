//! `shoal ls`: sessions › tabs › panes as a tree, with titles, directories
//! and status (M1 spec §11). Pure: the caller passes `$HOME` in.

use std::path::Path;

use shoal_client::model::{Pane, PaneStatus, Session, Tab, Workspace};

pub fn render(workspace: &Workspace, home: Option<&Path>) -> String {
    if workspace.sessions.is_empty() {
        return "No sessions.\n".into();
    }
    // Pane columns line up across the whole listing.
    let rows: Vec<_> = workspace
        .sessions
        .iter()
        .flat_map(|session| &session.tabs)
        .flat_map(|tab| panes(workspace, tab))
        .map(|pane| (label(pane), directory(pane, home)))
        .collect();
    let label_width = rows
        .iter()
        .map(|(label, _)| width(label))
        .max()
        .unwrap_or(0);
    let directory_width = rows.iter().map(|(_, dir)| width(dir)).max().unwrap_or(0);
    let mut out = String::new();
    for session in &workspace.sessions {
        out.push_str(&format!(
            "{} {}{}\n",
            session.id,
            session.name,
            windows(session)
        ));
        for (t, tab) in session.tabs.iter().enumerate() {
            let last_tab = t + 1 == session.tabs.len();
            let (branch, indent) = if last_tab {
                ("└─", "   ")
            } else {
                ("├─", "│  ")
            };
            let mut line = format!("{branch} {} {}", tab.id, tab_title(workspace, tab));
            if tab.id == session.active_tab {
                line.push_str(" (active)");
            }
            out.push_str(&line);
            out.push('\n');
            let tab_panes = panes(workspace, tab);
            for (p, pane) in tab_panes.iter().enumerate() {
                let branch = if p + 1 == tab_panes.len() {
                    "└─"
                } else {
                    "├─"
                };
                let (label, dir) = (label(pane), directory(pane, home));
                let mut line = format!(
                    "{indent}{branch} {label}{}  {dir}{}  {}",
                    pad(&label, label_width),
                    pad(&dir, directory_width),
                    status(&pane.status)
                );
                if tab_panes.len() > 1 && pane.id == tab.focused {
                    line.push_str(" (focused)");
                }
                if tab.zoomed == Some(pane.id) {
                    line.push_str(" (zoomed)");
                }
                out.push_str(&line);
                out.push('\n');
            }
        }
    }
    out
}

fn panes<'w>(workspace: &'w Workspace, tab: &Tab) -> Vec<&'w Pane> {
    let mut panes = Vec::new();
    tab.layout.for_each_pane(&mut |id| {
        if let Some(pane) = workspace.panes.get(&id) {
            panes.push(pane);
        }
    });
    panes
}

fn windows(session: &Session) -> String {
    match session.windows {
        0 => String::new(),
        1 => " (1 window)".into(),
        count => format!(" ({count} windows)"),
    }
}

/// The user's name for a tab, else its focused pane's title (§4.3).
fn tab_title(workspace: &Workspace, tab: &Tab) -> String {
    tab.name.clone().unwrap_or_else(|| {
        workspace
            .panes
            .get(&tab.focused)
            .map(|pane| pane.title.clone())
            .unwrap_or_default()
    })
}

fn label(pane: &Pane) -> String {
    format!("{} {}", pane.id, pane.title)
}

fn directory(pane: &Pane, home: Option<&Path>) -> String {
    let Some(cwd) = &pane.cwd else {
        return "-".into();
    };
    if let Some(rest) = home.and_then(|home| cwd.strip_prefix(home).ok()) {
        if rest.as_os_str().is_empty() {
            return "~".into();
        }
        return format!("~/{}", rest.display());
    }
    cwd.display().to_string()
}

fn status(status: &PaneStatus) -> String {
    match status {
        PaneStatus::Running => "running".into(),
        PaneStatus::Exited {
            code: Some(code), ..
        } => format!("exited {code}"),
        PaneStatus::Exited {
            signal: Some(signal),
            ..
        } => format!("killed by signal {signal}"),
        PaneStatus::Exited { .. } => "exited".into(),
    }
}

fn width(text: &str) -> usize {
    text.chars().count()
}

fn pad(text: &str, to: usize) -> String {
    " ".repeat(to.saturating_sub(width(text)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_workspace_says_so() {
        let workspace: Workspace = serde_json::from_value(
            serde_json::json!({"rev": 1, "next_id": 1, "sessions": [], "panes": {}}),
        )
        .unwrap();
        assert_eq!(render(&workspace, None), "No sessions.\n");
    }

    #[test]
    fn directories_under_home_use_a_tilde() {
        let home = Path::new("/home/parker");
        let pane = |cwd: Option<&str>| -> Pane {
            serde_json::from_value(serde_json::json!({
                "id": "p1", "kind": {"kind": "shell", "argv": []}, "size": {"cols": 80, "rows": 24},
                "cwd": cwd, "program": null, "title": "x", "status": {"state": "running"},
                "bell": false, "restored_from": null}))
            .unwrap()
        };
        assert_eq!(directory(&pane(Some("/home/parker")), Some(home)), "~");
        assert_eq!(
            directory(&pane(Some("/home/parker/code")), Some(home)),
            "~/code"
        );
        assert_eq!(
            directory(&pane(Some("/home/parkerx")), Some(home)),
            "/home/parkerx"
        );
        assert_eq!(directory(&pane(None), Some(home)), "-");
        assert_eq!(directory(&pane(Some("/tmp")), None), "/tmp");
    }

    #[test]
    fn statuses_read_plainly() {
        assert_eq!(status(&PaneStatus::Running), "running");
        let exited = |code, signal| status(&PaneStatus::Exited { code, signal });
        assert_eq!(exited(Some(0), None), "exited 0");
        assert_eq!(exited(None, Some(9)), "killed by signal 9");
        assert_eq!(exited(None, None), "exited");
    }
}
