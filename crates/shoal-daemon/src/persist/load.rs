//! Reading what the last daemon saved (M1 spec §6.4 steps 1, 2 and 7). Pure
//! apart from [`Store::load`] and [`Store::read_scrollback`], which block.

use std::{
    collections::BTreeMap,
    fs, io,
    io::Read as _,
    path::{Path, PathBuf},
};

use jiff::{Timestamp, tz::TimeZone};
use shoal_model::{GridSize, Layout, Pane, PaneId, PaneKind, PaneStatus, Session, Tab, Workspace};

use super::{Document, FORMAT, SavedPane, Store, scrollback_path};

/// The most a scrollback file may decompress to. Formatter output for 10,000
/// lines at 1,000 columns, styled, fits with room to spare.
const MAX_SCROLLBACK_BYTES: u64 = 256 * 1024 * 1024;

/// The terminal sizes a restored pane may have, as for snapshots (§5.5).
const SIZES: std::ops::RangeInclusive<u16> = 1..=1000;

/// What restore starts from.
#[derive(Debug)]
pub enum Loaded {
    /// No saved workspace: start empty.
    Empty,
    /// The file could not be used. It was moved aside to `moved_to`, unless
    /// that failed too; the daemon starts empty either way.
    Broken {
        reason: String,
        moved_to: Option<PathBuf>,
    },
    Restored(Restored),
}

/// A saved workspace, valid and ready for its panes to start.
#[derive(Debug, Clone, PartialEq)]
pub struct Restored {
    /// Every pane a `Running` shell with no argv or program, `restored_from`
    /// the save time; `rev` 1, and `next_id` above every id in it.
    pub workspace: Workspace,
    pub saved_at: Timestamp,
    pub panes: BTreeMap<PaneId, SavedPane>,
}

impl Store {
    /// Read `workspace.json`. An unreadable, unknown or invalid file is
    /// renamed `workspace.json.broken-<time>` (step 2).
    pub fn load(&self, now: Timestamp) -> Loaded {
        let reason = match fs::read(&self.workspace) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Loaded::Empty,
            Err(error) => error.to_string(),
            Ok(bytes) => match parse(&bytes) {
                Ok(restored) => return Loaded::Restored(restored),
                Err(reason) => reason,
            },
        };
        let moved_to = broken_name(&self.workspace, now);
        let moved_to = match fs::rename(&self.workspace, &moved_to) {
            Ok(()) => Some(moved_to),
            Err(error) => {
                tracing::warn!(%error, "could not move the broken workspace aside");
                None
            }
        };
        Loaded::Broken { reason, moved_to }
    }

    /// A pane's saved terminal bytes, decompressed.
    pub fn read_scrollback(&self, pane: PaneId) -> io::Result<Vec<u8>> {
        let file = fs::File::open(self.state.join(scrollback_path(pane)))?;
        let mut bytes = Vec::new();
        zstd::stream::read::Decoder::new(file)?
            .take(MAX_SCROLLBACK_BYTES + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_SCROLLBACK_BYTES {
            return Err(io::Error::other("scrollback is too large"));
        }
        Ok(bytes)
    }
}

/// A saved document as a live workspace, or why it can't be one.
pub fn parse(bytes: &[u8]) -> Result<Restored, String> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| error.to_string())?;
    match value.get("format").and_then(serde_json::Value::as_u64) {
        Some(format) if format == u64::from(FORMAT) => {}
        Some(format) => return Err(format!("unknown format {format}")),
        None => return Err("no format number".into()),
    }
    let document: Document = serde_json::from_value(value).map_err(|error| error.to_string())?;
    restore(document)
}

fn restore(document: Document) -> Result<Restored, String> {
    if let Some((id, pane)) = document
        .panes
        .iter()
        .find(|(_, pane)| !SIZES.contains(&pane.size.cols) || !SIZES.contains(&pane.size.rows))
    {
        let GridSize { cols, rows } = pane.size;
        return Err(format!("pane {id} has an impossible size {cols} × {rows}"));
    }
    // A wrong counter must never hand out an id that is already in use.
    let next_id = max_id(&document)
        .map_or(1, |id| id.saturating_add(1))
        .max(document.next_id);
    let workspace = Workspace {
        rev: 1,
        next_id,
        sessions: document
            .sessions
            .into_iter()
            .map(|session| Session {
                id: session.id,
                name: session.name,
                tabs: session
                    .tabs
                    .into_iter()
                    .map(|tab| Tab {
                        id: tab.id,
                        name: tab.name,
                        layout: tab.layout,
                        focused: tab.focused,
                        zoomed: tab.zoomed,
                    })
                    .collect(),
                active_tab: session.active_tab,
                created_at: session.created_at,
                last_active_at: session.last_active_at,
                windows: 0,
            })
            .collect(),
        panes: document
            .panes
            .iter()
            .map(|(&id, saved)| {
                // A fresh shell, never the saved command again (step 6).
                let pane = Pane {
                    id,
                    kind: PaneKind::Shell { argv: Vec::new() },
                    size: saved.size,
                    cwd: saved.cwd.clone(),
                    // Nothing runs yet; the divider says what did.
                    program: None,
                    title: String::new(),
                    status: PaneStatus::Running,
                    bell: false,
                    restored_from: Some(document.saved_at),
                };
                (id, pane)
            })
            .collect(),
    };
    shoal_model::validate(&workspace).map_err(|error| error.to_string())?;
    Ok(Restored {
        workspace,
        saved_at: document.saved_at,
        panes: document.panes,
    })
}

fn max_id(document: &Document) -> Option<u64> {
    fn layout_ids(layout: &Layout, ids: &mut Vec<u64>) {
        match layout {
            Layout::Pane(pane) => ids.push(pane.get()),
            Layout::Split {
                id, first, second, ..
            } => {
                ids.push(id.get());
                layout_ids(first, ids);
                layout_ids(second, ids);
            }
        }
    }
    let mut ids: Vec<u64> = document.panes.keys().map(|id| id.get()).collect();
    for session in &document.sessions {
        ids.push(session.id.get());
        for tab in &session.tabs {
            ids.push(tab.id.get());
            layout_ids(&tab.layout, &mut ids);
        }
    }
    ids.into_iter().max()
}

fn broken_name(workspace: &Path, now: Timestamp) -> PathBuf {
    let mut name = workspace.as_os_str().to_owned();
    name.push(format!(".broken-{}", now.strftime("%Y%m%dT%H%M%SZ")));
    PathBuf::from(name)
}

/// The divider under a restored pane's old output (step 5), such as
/// `── restored · Fri Oct 24 14:02 · was running nvim ──`, in local time.
pub fn divider(
    saved_at: Timestamp,
    zone: &TimeZone,
    program: Option<&str>,
    folder_gone: bool,
    unreadable: bool,
) -> String {
    let time = saved_at.to_zoned(zone.clone()).strftime("%a %b %-d %H:%M");
    let mut parts = vec!["restored".to_owned(), time.to_string()];
    if let Some(program) = program.filter(|program| !program.is_empty()) {
        parts.push(format!("was running {program}"));
    }
    if folder_gone {
        parts.push("(folder gone)".into());
    }
    if unreadable {
        parts.push("(old output unreadable)".into());
    }
    format!("── {} ──", parts.join(" · "))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Written by the T15 daemon (the reviewed `workspace.json` snapshot),
    /// with real times and paths, and `p3` running nvim.
    const FORMAT_ONE: &str = include_str!("../../tests/fixtures/workspace-format-1.json");

    fn edited(edit: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
        let mut value: serde_json::Value = serde_json::from_str(FORMAT_ONE).unwrap();
        edit(&mut value);
        serde_json::to_vec(&value).unwrap()
    }

    #[test]
    fn a_saved_format_one_file_loads_as_the_workspace_it_describes() {
        let restored = parse(FORMAT_ONE.as_bytes()).unwrap();
        let saved_at: Timestamp = "2026-10-24T14:02:11Z".parse().unwrap();
        assert_eq!(restored.saved_at, saved_at);
        let workspace = &restored.workspace;
        assert_eq!((workspace.rev, workspace.next_id), (1, 8));
        let [session] = &workspace.sessions[..] else {
            panic!("{workspace:?}")
        };
        assert_eq!((session.name.as_str(), session.windows), ("main", 0));
        assert_eq!(session.active_tab.to_string(), "t6");
        let tabs: Vec<_> = session
            .tabs
            .iter()
            .map(|tab| {
                (
                    tab.id.to_string(),
                    tab.name.clone(),
                    tab.focused.to_string(),
                )
            })
            .collect();
        assert_eq!(
            tabs,
            [
                ("t2".into(), None, "p5".into()),
                ("t6".into(), Some("waiting".into()), "p7".into())
            ]
        );
        assert_eq!(session.tabs[0].zoomed, Some(PaneId::new(5)));
        let Layout::Split { ratio, .. } = &session.tabs[0].layout else {
            panic!("a split")
        };
        assert_eq!(*ratio, 0.6);
        let p3 = &workspace.panes[&PaneId::new(3)];
        assert_eq!(p3.cwd.as_deref(), Some(Path::new("/home/parker/project")));
        assert_eq!(p3.program, None);
        assert_eq!(
            restored.panes[&PaneId::new(3)].program.as_deref(),
            Some("nvim")
        );
        assert_eq!(p3.size, GridSize { cols: 80, rows: 24 });
        for pane in workspace.panes.values() {
            assert_eq!(pane.status, PaneStatus::Running);
            assert_eq!(pane.restored_from, Some(saved_at));
            // The `sleep 1000` pane comes back as a shell.
            assert_eq!(pane.kind, PaneKind::Shell { argv: Vec::new() });
        }
        assert_eq!(
            restored.panes[&PaneId::new(7)].scrollback.as_deref(),
            Some("panes/p7/scrollback.vt.zst")
        );
    }

    #[test]
    fn new_ids_start_above_every_restored_id_even_with_a_wrong_counter() {
        for (next_id, want) in [(2, 8), (8, 8), (57, 57)] {
            let bytes = edited(|value| value["next_id"] = next_id.into());
            assert_eq!(parse(&bytes).unwrap().workspace.next_id, want, "{next_id}");
        }
        assert_eq!(
            parse(br#"{"format":1,"saved_at":"2026-10-24T14:02:11Z","shoal_version":"0.1.0","next_id":0,"sessions":[],"panes":{}}"#)
                .unwrap()
                .workspace
                .next_id,
            1,
            "ids start at 1",
        );
    }

    #[test]
    fn unusable_files_are_rejected_with_a_reason() {
        let cases: [(&str, Vec<u8>, &str); 6] = [
            ("not json", b"{\"format\": 1, \"sess".to_vec(), "EOF"),
            (
                "newer format",
                edited(|v| v["format"] = 2.into()),
                "unknown format 2",
            ),
            (
                "no format",
                edited(|v| {
                    v.as_object_mut().unwrap().remove("format");
                }),
                "no format",
            ),
            (
                "wrong shape",
                edited(|v| v["sessions"] = "main".into()),
                "invalid type",
            ),
            (
                "missing pane",
                edited(|v| {
                    v["panes"].as_object_mut().unwrap().remove("p7");
                }),
                "p7",
            ),
            (
                "zero size",
                edited(|v| v["panes"]["p3"]["size"]["cols"] = 0.into()),
                "impossible size",
            ),
        ];
        for (case, bytes, reason) in cases {
            let error = parse(&bytes).expect_err(case);
            assert!(error.contains(reason), "{case}: {error}");
        }
    }

    #[test]
    fn a_broken_file_is_moved_aside_and_a_missing_one_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store {
            state: dir.path().into(),
            workspace: dir.path().join("workspace.json"),
            panes: dir.path().join("panes"),
        };
        let now: Timestamp = "2026-10-25T20:30:14Z".parse().unwrap();
        assert!(matches!(store.load(now), Loaded::Empty));
        fs::write(&store.workspace, "garbage").unwrap();
        let Loaded::Broken { reason, moved_to } = store.load(now) else {
            panic!("broken")
        };
        assert!(reason.contains("expected value"), "{reason}");
        let moved_to = moved_to.unwrap();
        assert_eq!(
            moved_to,
            dir.path().join("workspace.json.broken-20261025T203014Z")
        );
        assert_eq!(fs::read(&moved_to).unwrap(), b"garbage");
        assert!(!store.workspace.exists());
        fs::write(&store.workspace, FORMAT_ONE).unwrap();
        assert!(matches!(store.load(now), Loaded::Restored(_)));
    }

    #[test]
    fn scrollback_reads_decompress_and_refuse_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store {
            state: dir.path().into(),
            workspace: dir.path().join("workspace.json"),
            panes: dir.path().join("panes"),
        };
        let pane = PaneId::new(4);
        assert_eq!(
            store.read_scrollback(pane).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        store
            .write_scrollback(pane, &zstd::bulk::compress(b"old output", 3).unwrap())
            .unwrap();
        assert_eq!(store.read_scrollback(pane).unwrap(), b"old output");
        store.write_scrollback(pane, b"not zstd").unwrap();
        assert!(store.read_scrollback(pane).is_err());
    }

    #[test]
    fn the_divider_names_the_time_program_and_problems() {
        let saved_at: Timestamp = "2026-10-24T14:02:11Z".parse().unwrap();
        let utc = TimeZone::UTC;
        assert_eq!(
            divider(saved_at, &utc, Some("nvim"), false, false),
            "── restored · Sat Oct 24 14:02 · was running nvim ──"
        );
        assert_eq!(
            divider(saved_at, &utc, None, true, true),
            "── restored · Sat Oct 24 14:02 · (folder gone) · (old output unreadable) ──"
        );
        let early: Timestamp = "2026-10-05T03:04:00Z".parse().unwrap();
        let zone = TimeZone::fixed(jiff::tz::offset(-7));
        assert_eq!(
            divider(early, &zone, Some(""), false, false),
            "── restored · Sun Oct 4 20:04 ──"
        );
    }
}
