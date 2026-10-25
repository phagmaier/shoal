mod common;

use serde_json::json;
use shoal_model::{Axis, Layout, PaneKind, PaneStatus, Workspace};

use common::{leaf, split, workspace};

#[test]
fn workspace_json_snapshot() {
    let json = serde_json::to_string_pretty(&workspace()).expect("workspace serializes");
    insta::assert_snapshot!(json);
}

#[test]
fn workspace_round_trips() {
    let original = workspace();
    let json = serde_json::to_string(&original).expect("workspace serializes");
    let parsed: Workspace = serde_json::from_str(&json).expect("workspace deserializes");
    assert_eq!(parsed, original);
}

/// The shapes written out in M1 spec §6.2.
#[test]
fn reads_spec_example_shapes() {
    let layout: Layout = serde_json::from_value(json!({
        "split": { "id": "x3", "axis": "row", "ratio": 0.6,
                   "first": { "pane": "p4" }, "second": { "pane": "p5" } }
    }))
    .expect("spec layout parses");
    assert_eq!(layout, split(3, Axis::Row, 0.6, leaf(4), leaf(5)));

    let kind: PaneKind =
        serde_json::from_value(json!({ "kind": "shell", "argv": [] })).expect("kind parses");
    assert_eq!(kind, PaneKind::Shell { argv: Vec::new() });
}

#[test]
fn status_is_tagged_by_state() {
    assert_eq!(
        serde_json::to_value(PaneStatus::Running).expect("serializes"),
        json!({ "state": "running" })
    );
    assert_eq!(
        serde_json::to_value(PaneStatus::Exited {
            code: Some(3),
            signal: None
        })
        .expect("serializes"),
        json!({ "state": "exited", "code": 3, "signal": null })
    );
}

#[test]
fn rejects_id_of_wrong_kind() {
    let mut value = serde_json::to_value(workspace()).expect("serializes");
    value["sessions"][0]["tabs"][0]["focused"] = json!("t6");
    let err = serde_json::from_value::<Workspace>(value).expect_err("t6 is not a pane id");
    assert!(err.to_string().contains("invalid pane id \"t6\""), "{err}");
}

#[test]
fn rejects_unprefixed_pane_key() {
    let mut value = serde_json::to_value(workspace()).expect("serializes");
    let panes = value["panes"].as_object_mut().expect("panes is an object");
    let p4 = panes.remove("p4").expect("p4 present");
    panes.insert("4".into(), p4);
    let err = serde_json::from_value::<Workspace>(value).expect_err("4 is not a pane id");
    assert!(err.to_string().contains("invalid pane id \"4\""), "{err}");
}
