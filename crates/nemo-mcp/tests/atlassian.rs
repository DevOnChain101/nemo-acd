//! Confluence/Jira-like server (examples/atlassian_like_server): what survives the client.
mod common;
use common::*;
use nemo_mcp::{McpManager, McpServerConfig};
use serde_json::{Value, json};

fn cfg(flags: &[&str]) -> McpServerConfig {
    McpServerConfig::stdio(
        "atl",
        example_path("atlassian_like_server").to_string_lossy(),
        flags.iter().map(|s| s.to_string()).collect(),
    )
}

#[tokio::test]
async fn annotations_structured_content_and_version_survive() {
    let m = McpManager::new(vec![cfg(&["--instructions"])], opts(base_env()));
    m.start().await;
    let tools = m.tools();
    let ann = |n: &str| {
        tools
            .iter()
            .find(|t| t.tool == n)
            .unwrap()
            .annotations
            .clone()
    };
    assert_eq!(ann("getPage").read_only, Some(true));
    assert_eq!(
        (
            ann("updatePage").read_only,
            ann("updatePage").destructive,
            ann("editIssue").idempotent
        ),
        (Some(false), Some(true), Some(true))
    );
    assert!(tools[0].description.contains("untrusted data"));
    let r = call(&m, "atl", "getPage", json!({"id": "100"})).await;
    assert_eq!(r.structured.as_ref().unwrap()["version"]["number"], 3);
    let v: Value = serde_json::from_str(&r.text()).unwrap();
    assert_eq!(v["version"]["number"], 3);
    m.close().await;
}

#[tokio::test]
async fn a_60k_page_keeps_id_and_version_and_is_saved_in_full() {
    let dir = tempfile::tempdir().unwrap();
    let mut o = opts(base_env());
    o.artifact_dir = Some(dir.path().to_path_buf());
    let m = McpManager::new(vec![cfg(&["--big-body"])], o);
    m.start().await;
    let r = call(&m, "atl", "getPage", json!({"id": "100"})).await;
    let text = r.text();
    assert!(
        text.chars().count() <= 28_000 && text.contains("full result saved to"),
        "{}",
        text.len()
    );
    assert!(text.contains(r#""version":{"number":3}"#) && text.contains(r#""id":"100""#));
    assert!(r.structured.unwrap()["body"].as_str().unwrap().len() > 60_000);
    let saved = std::fs::read_dir(dir.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert!(std::fs::read_to_string(saved).unwrap().len() > 60_000);
    m.close().await;
}

#[tokio::test]
async fn a_lost_connection_is_re_established_with_backoff() {
    let mut o = opts(base_env());
    o.reconnect_backoff = vec![std::time::Duration::from_millis(50); 3];
    let m = McpManager::new(vec![echo_cfg("echo", &[])], o);
    m.start().await;
    assert!(call(&m, "echo", "crash", json!({})).await.is_error);
    wait_until("auto reconnect", || {
        m.status()[0].state == nemo_mcp::ServerState::Connected
    })
    .await;
    assert_eq!(
        call(&m, "echo", "echo", json!({"text": "back"}))
            .await
            .text(),
        "back"
    );
    m.close().await;
}
