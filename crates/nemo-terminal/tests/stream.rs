//! Live output (`ExecSpec::on_output`) and the explicit unconfined switch.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::fs;
use std::sync::{Arc, Mutex};

use nemo_core::WorkspaceContext;
use nemo_terminal::{Error, ExecSpec, OutputSink, SandboxMode, TerminalRuntime};
use tokio_util::sync::CancellationToken;

const SID: &str = "s";

fn open(mode: SandboxMode) -> (tempfile::TempDir, TerminalRuntime) {
    let dir = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(dir.path()).unwrap();
    fs::create_dir_all(base.join("w")).unwrap();
    let ws = WorkspaceContext::open(
        "w",
        &base.join("w"),
        &base.join("w"),
        &[],
        base.join("art"),
        "pg",
    )
    .unwrap();
    let rt = TerminalRuntime::new(mode);
    let env = BTreeMap::from([("PATH".to_string(), std::env::var("PATH").unwrap())]);
    rt.open_session(SID, Arc::new(ws), env);
    (dir, rt)
}

#[tokio::test]
async fn on_output_streams_utf8_safe_chunks_and_the_rest_at_the_end() {
    let (_d, rt) = open(SandboxMode::Off);
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let s2 = seen.clone();
    let mut spec =
        ExecSpec::new("printf 'h\\303'; sleep 0.2; printf '\\251llo\\n'; sleep 0.2; echo end");
    spec.on_output = Some(OutputSink(Arc::new(move |t| {
        s2.lock().unwrap().push(t.to_string())
    })));
    let r = rt.exec(SID, spec, &CancellationToken::new()).await.unwrap();
    let chunks = seen.lock().unwrap().clone();
    assert!(chunks.len() >= 2, "output arrived live: {chunks:?}");
    assert_eq!(chunks.concat(), "h\u{e9}llo\nend\n");
    assert_eq!(r.output, "h\u{e9}llo\nend\n");
}

#[tokio::test]
async fn unconfined_runs_without_the_session_policy_and_says_so() {
    let (_d, rt) = open(SandboxMode::Enforce);
    let e = rt
        .exec(SID, ExecSpec::new("echo hi"), &CancellationToken::new())
        .await;
    assert!(matches!(e, Err(Error::SandboxUnavailable(_))), "{e:?}");
    let mut spec = ExecSpec::new("echo hi");
    spec.unconfined = true;
    let r = rt.exec(SID, spec, &CancellationToken::new()).await.unwrap();
    assert_eq!(r.output.trim(), "hi");
    assert!(r.sandbox.is_none() && r.unsandboxed_by_request);
}
