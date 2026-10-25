use std::collections::BTreeMap;

use shoal_model::{
    Axis, GridSize, Layout, Pane, PaneId, PaneKind, PaneStatus, Session, SessionId, SplitId, Tab,
    TabId, Timestamp, Workspace,
};

pub fn at(rfc3339: &str) -> Timestamp {
    rfc3339.parse().expect("fixture timestamp is valid")
}

pub fn pane(id: u64, program: Option<&str>, title: &str, status: PaneStatus) -> Pane {
    Pane {
        id: PaneId::new(id),
        kind: PaneKind::Shell { argv: Vec::new() },
        size: GridSize { cols: 80, rows: 24 },
        cwd: Some("/home/parker/code/shoal".into()),
        program: program.map(str::to_owned),
        title: title.to_owned(),
        status,
        bell: false,
        restored_from: None,
    }
}

pub fn split(id: u64, axis: Axis, ratio: f32, first: Layout, second: Layout) -> Layout {
    Layout::Split {
        id: SplitId::new(id),
        axis,
        ratio,
        first: Box::new(first),
        second: Box::new(second),
    }
}

pub fn leaf(id: u64) -> Layout {
    Layout::Pane(PaneId::new(id))
}

/// A valid workspace with two sessions, nested splits and a zoomed pane.
///
/// - s1 "main": t2 with x3 = p4 | (x5 = p6 / p7), p6 focused and zoomed;
///   t8 "logs" with p9 running htop, restored after a reboot.
/// - s10 "2": t11 with p12.
///
/// Ids 13 to 19 belonged to closed items, so `next_id` is 20.
pub fn workspace() -> Workspace {
    let mut p4 = pane(4, Some("nvim"), "nvim", PaneStatus::Running);
    p4.size = GridSize {
        cols: 120,
        rows: 40,
    };
    let p6 = pane(6, None, "shoal", PaneStatus::Running);
    let mut p7 = pane(
        7,
        None,
        "shoal",
        PaneStatus::Exited {
            code: Some(0),
            signal: None,
        },
    );
    p7.bell = true;
    let mut p9 = pane(9, Some("htop"), "htop", PaneStatus::Running);
    p9.kind = PaneKind::Shell {
        argv: vec!["htop".into()],
    };
    p9.cwd = None;
    p9.restored_from = Some(at("2026-10-24T14:02:11Z"));
    let p12 = pane(
        12,
        None,
        "~",
        PaneStatus::Exited {
            code: None,
            signal: Some(9),
        },
    );

    let main = Session {
        id: SessionId::new(1),
        name: "main".into(),
        tabs: vec![
            Tab {
                id: TabId::new(2),
                name: None,
                layout: split(
                    3,
                    Axis::Row,
                    0.6,
                    leaf(4),
                    split(5, Axis::Column, 0.5, leaf(6), leaf(7)),
                ),
                focused: PaneId::new(6),
                zoomed: Some(PaneId::new(6)),
            },
            Tab {
                id: TabId::new(8),
                name: Some("logs".into()),
                layout: leaf(9),
                focused: PaneId::new(9),
                zoomed: None,
            },
        ],
        active_tab: TabId::new(2),
        created_at: at("2026-10-20T09:00:00Z"),
        last_active_at: at("2026-10-24T14:01:58Z"),
        windows: 1,
    };
    let second = Session {
        id: SessionId::new(10),
        name: "2".into(),
        tabs: vec![Tab {
            id: TabId::new(11),
            name: None,
            layout: leaf(12),
            focused: PaneId::new(12),
            zoomed: None,
        }],
        active_tab: TabId::new(11),
        created_at: at("2026-10-24T08:30:00.5Z"),
        last_active_at: at("2026-10-24T08:30:00.5Z"),
        windows: 0,
    };

    Workspace {
        rev: 42,
        next_id: 20,
        sessions: vec![main, second],
        panes: BTreeMap::from([p4, p6, p7, p9, p12].map(|pane| (pane.id, pane))),
    }
}
