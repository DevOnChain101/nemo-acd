//! `meta.diff` of a file-changing tool reaches `tool.completed.diff` (additive; redacted).
mod common;

use common::*;
use nemo_events::Event;
use nemo_harness::ToolBox;
use nemo_harness::testing::Turn;
use nemo_tools::Registry;
use serde_json::json;

#[tokio::test]
async fn write_and_edit_events_carry_a_line_numbered_diff() {
    let mut rig = Rig::new(
        Opts {
            tools: ToolBox::new(Registry::builtin()),
            allow: vec!["Write".into(), "Read".into(), "Edit".into()],
            ..Opts::default()
        },
        vec![
            Turn::tool(
                "w1",
                "Write",
                json!({"file_path": "a.txt", "content": "one\ntwo\nthree\n"}),
            ),
            Turn::tool("r1", "Read", json!({"file_path": "a.txt"})),
            Turn::tool(
                "e1",
                "Edit",
                json!({"file_path": "a.txt", "old_string": "two", "new_string": "TWO"}),
            ),
            Turn::text("done"),
        ],
    );
    rig.run("make a file").await;
    let done = rig.find(|e| match e {
        Event::ToolCompleted { name, diff, .. } => Some((name.clone(), diff.clone())),
        _ => None,
    });
    let get = |n: &str| done.iter().find(|(x, _)| x == n).unwrap().1.clone();
    let w = get("Write").expect("Write carries a diff");
    assert_eq!(
        (w["kind"].clone(), w["added"].clone()),
        (json!("create"), json!(3))
    );
    let e = get("Edit").expect("Edit carries a diff");
    assert_eq!(e["kind"], "update");
    let lines = e["hunks"][0]["lines"].as_array().unwrap();
    assert!(
        lines
            .iter()
            .any(|l| l["tag"] == "-" && l["old_no"] == 2 && l["text"] == "two"),
        "{e}"
    );
    assert!(
        lines
            .iter()
            .any(|l| l["tag"] == "+" && l["new_no"] == 2 && l["text"] == "TWO"),
        "{e}"
    );
    assert!(get("Read").is_none(), "other tools have no diff");
}
