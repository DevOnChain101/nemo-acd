use nemo_core::WorkspaceContext;
use nemo_events::{Event, TaskStatus};
use nemo_terminal::{SandboxMode, TerminalRuntime};
use nemo_verification::*;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

const SID: &str = "s1";

struct Fx {
    _dir: tempfile::TempDir,
    rt: Arc<TerminalRuntime>,
    ws: Arc<WorkspaceContext>,
}

fn fx_path(files: &[(&str, &str)], path: Option<&str>) -> Fx {
    let dir = tempfile::tempdir().unwrap();
    let base = nemo_core::canonicalize(dir.path()).unwrap();
    let w = base.join("w");
    fs::create_dir_all(&w).unwrap();
    for (n, c) in files {
        fs::write(w.join(n), c).unwrap();
    }
    let ws = Arc::new(WorkspaceContext::open("w", &w, &w, &[], base.join("art"), "pg").unwrap());
    let mut env = BTreeMap::new();
    let host_path = std::env::var("PATH").unwrap();
    env.insert("PATH".to_string(), path.map_or(host_path, Into::into));
    // Windows needs its own set for cargo/node/python to start under an otherwise empty env.
    let win = [
        "SystemRoot",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "TEMP",
        "TMP",
        "PATHEXT",
        "ComSpec",
        "HOMEDRIVE",
        "HOMEPATH",
        // rustc locates the MSVC linker through these; without them it falls back to Git's `link`.
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramData",
        "windir",
        "SystemDrive",
        "LIB",
        "INCLUDE",
    ];
    for k in ["HOME", "CARGO_HOME", "RUSTUP_HOME", "RUSTUP_TOOLCHAIN"]
        .into_iter()
        .chain(win.into_iter().filter(|_| cfg!(windows)))
    {
        if let Ok(v) = std::env::var(k) {
            env.insert(k.into(), v);
        }
    }
    let rt = Arc::new(TerminalRuntime::new(SandboxMode::Off));
    rt.open_session(SID, ws.clone(), env);
    Fx { _dir: dir, rt, ws }
}

fn fx(files: &[(&str, &str)]) -> Fx {
    fx_path(files, None)
}

impl Fx {
    fn gate(&self, cfg: GateConfig) -> CompletionGate {
        CompletionGate::new(
            VerifyRunner::new(self.rt.clone(), SID, self.ws.clone()),
            cfg,
        )
    }
    fn runner(&self) -> VerifyRunner {
        VerifyRunner::new(self.rt.clone(), SID, self.ws.clone())
    }
}

fn check(f: &Fx, id: &str, cmd: &str, timeout_ms: u64) -> Check {
    Check {
        id: id.into(),
        kind: CheckKind::Custom,
        command: Some(cmd.into()),
        cwd: f.ws.cwd.clone(),
        timeout_ms,
        declared_by: Declarer::Model,
        criteria_ids: vec![],
    }
}

fn state(criteria: Vec<Criterion>, changed: &[&str]) -> TaskState {
    TaskState {
        task_id: "t1".into(),
        session_id: SID.into(),
        criteria,
        side_effects: true,
        changed_files: changed.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

fn have(tool: &str) -> bool {
    std::process::Command::new(tool)
        .arg("--version")
        .output()
        .is_ok()
}

const NODE_OK: &str =
    "const t=require('node:test');const a=require('node:assert');t('a',()=>a.equal(1,1));";
const NODE_BAD: &str =
    "const t=require('node:test');const a=require('node:assert');t('a',()=>a.equal(1,2));";
const PY_OK: &str =
    "import unittest\nclass T(unittest.TestCase):\n    def test_a(self): self.assertEqual(1, 1)\n";
const PY_BAD: &str =
    "import unittest\nclass T(unittest.TestCase):\n    def test_a(self): self.assertEqual(1, 2)\n";

fn node_criteria() -> Vec<Criterion> {
    vec![Criterion::user("c1", "math works", Some("node --test"))]
}

#[tokio::test]
async fn node_pass_completes_with_evidence() {
    if !have("node") {
        return;
    }
    let f = fx(&[("a.test.js", NODE_OK)]);
    let mut g = f.gate(GateConfig::default());
    let d = g
        .evaluate(
            &state(node_criteria(), &["a.test.js"]),
            true,
            &CancellationToken::new(),
        )
        .await;
    let GateDecision::Complete(Some(r)) = d else {
        panic!("{d:?}")
    };
    assert_eq!(r.status, VerificationStatus::Passed);
    assert_eq!(r.criteria[0].status, VerificationStatus::Passed);
    assert_eq!(r.criteria[0].evidence, ["k1"]);
    let k = &r.checks[0];
    assert_eq!(k.exit_code, Some(0));
    assert_eq!(k.artifacts.len(), 1);
    assert!(f.ws.artifact_root.join(&k.artifacts[0].path).is_file());
    assert_eq!(k.artifacts[0].sha256.len(), 64);
}

#[tokio::test]
async fn node_fail_gives_feedback_then_no_progress_stop() {
    if !have("node") {
        return;
    }
    let f = fx(&[("a.test.js", NODE_BAD)]);
    let mut g = f.gate(GateConfig::default());
    let st = state(node_criteria(), &["a.test.js"]);
    let c = CancellationToken::new();
    let GateDecision::Continue { feedback, report } = g.evaluate(&st, true, &c).await else {
        panic!("expected Continue")
    };
    assert_eq!(report.status, VerificationStatus::Failed);
    assert!(feedback.contains("FAILED k1"), "{feedback}");
    assert!(feedback.contains("NOT verified"));
    assert!(!report.checks[0].failure_fingerprint.is_none());
    // same failure, same changed files => stop
    let GateDecision::Stop { status, reason, .. } = g.evaluate(&st, true, &c).await else {
        panic!("expected Stop")
    };
    assert_eq!(
        (status, reason.as_str()),
        (TaskStatus::Failed, "no_progress")
    );
}

#[tokio::test]
async fn budget_stop_when_progress_is_made_but_attempts_run_out() {
    if !have("node") {
        return;
    }
    let f = fx(&[("a.test.js", NODE_BAD)]);
    let cfg = GateConfig {
        max_verify_attempts: 2,
        ..Default::default()
    };
    let mut g = f.gate(cfg);
    let c = CancellationToken::new();
    assert!(matches!(
        g.evaluate(&state(node_criteria(), &["a"]), true, &c).await,
        GateDecision::Continue { .. }
    ));
    let GateDecision::Stop {
        status,
        reason,
        report,
    } = g
        .evaluate(&state(node_criteria(), &["a", "b"]), true, &c)
        .await
    else {
        panic!("expected Stop")
    };
    assert_eq!(
        (status, reason.as_str()),
        (TaskStatus::Failed, "verify_budget_exhausted")
    );
    assert_eq!(report.unwrap().attempt, 2);
}

#[tokio::test]
async fn python_unittest_inferred_pass_and_fail() {
    if !have("python3") {
        return;
    }
    let f = fx(&[("test_a.py", PY_BAD)]);
    let mut g = f.gate(GateConfig::default());
    let c = CancellationToken::new();
    let GateDecision::Continue { report, .. } = g.evaluate(&state(vec![], &[]), true, &c).await
    else {
        panic!("expected Continue")
    };
    assert_eq!(report.status, VerificationStatus::Failed);
    assert!(
        report.checks[0]
            .command
            .as_deref()
            .unwrap()
            .contains("unittest discover")
    );

    // Inferred-only pass: capped at unverified by default, one correction round, then stop.
    let f = fx(&[("test_a.py", PY_OK)]);
    let mut g = f.gate(GateConfig::default());
    let GateDecision::Continue { report, feedback } =
        g.evaluate(&state(vec![], &[]), true, &c).await
    else {
        panic!("expected Continue")
    };
    assert_eq!(report.status, VerificationStatus::Unverified);
    assert!(
        report
            .limitations
            .iter()
            .any(|l| l.contains("inferred checks only"))
    );
    assert!(feedback.contains("verify_run"));
    let GateDecision::Stop { status, .. } = g.evaluate(&state(vec![], &[]), true, &c).await else {
        panic!("expected Stop")
    };
    assert_eq!(status, TaskStatus::Unverified);

    // Policy switch: spec rule 4 literally.
    let cfg = GateConfig {
        inferred_only: InferredOnlyPolicy::AllowPassed,
        ..Default::default()
    };
    let mut g = f.gate(cfg);
    assert!(matches!(
        g.evaluate(&state(vec![], &[]), true, &c).await,
        GateDecision::Complete(Some(r)) if r.status == VerificationStatus::Passed
    ));
}

#[tokio::test]
async fn python_user_criterion_passes() {
    if !have("python3") {
        return;
    }
    let f = fx(&[("test_a.py", PY_OK)]);
    let crit = vec![Criterion::user(
        "c1",
        "a holds",
        Some("python3 -m unittest"),
    )];
    let mut g = f.gate(GateConfig::default());
    let d = g
        .evaluate(&state(crit, &[]), true, &CancellationToken::new())
        .await;
    assert!(matches!(d, GateDecision::Complete(Some(r)) if r.status == VerificationStatus::Passed));
}

#[tokio::test]
async fn cargo_test_offline_pass_and_fail() {
    if !have("cargo") {
        return;
    }
    for (body, want) in [
        ("assert_eq!(1, 1)", VerificationStatus::Passed),
        ("assert_eq!(1, 2)", VerificationStatus::Failed),
    ] {
        let src = format!("#[test]\nfn t() {{ {body} }}\n");
        let f = fx(&[
            (
                "Cargo.toml",
                "[package]\nname=\"fx\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
            ),
            ("lib.rs", &src),
        ]);
        fs::write(
            f.ws.cwd.join("Cargo.toml"),
            "[package]\nname=\"fx\"\nversion=\"0.1.0\"\nedition=\"2021\"\n[lib]\npath=\"lib.rs\"\n",
        )
        .unwrap();
        let k = check(&f, "k1", "cargo test --offline", 300_000);
        let r = f.runner().run_one(&k, &CancellationToken::new()).await;
        assert_eq!(r.status, want, "{}", r.summary);
    }
}

#[tokio::test]
async fn timeout_is_failed() {
    let f = fx(&[]);
    let r = f
        .runner()
        .run_one(&check(&f, "k1", "sleep 30", 300), &CancellationToken::new())
        .await;
    assert_eq!(r.status, VerificationStatus::Failed);
    assert!(r.timed_out);
    assert!(r.duration_ms < 10_000);
}

#[tokio::test]
async fn missing_binary_is_blocked_not_failed() {
    let f = fx(&[]);
    let r = f
        .runner()
        .run_one(
            &check(&f, "k1", "definitely-not-a-real-tool-xyz --x", 5000),
            &CancellationToken::new(),
        )
        .await;
    assert_eq!(r.status, VerificationStatus::Blocked);
    assert!(r.blocked_reason.unwrap().contains("tool missing"));
    assert!(r.failure_fingerprint.is_none());

    // Each language tool, when it is absent from PATH, is blocked; gate stops `blocked`.
    let f = fx_path(
        &[("a.test.js", NODE_OK), ("test_a.py", PY_OK)],
        Some("/nonexistent"),
    );
    for cmd in ["node --test", "python3 -m unittest", "cargo test --offline"] {
        let r = f
            .runner()
            .run_one(&check(&f, "k1", cmd, 5000), &CancellationToken::new())
            .await;
        assert_eq!(
            r.status,
            VerificationStatus::Blocked,
            "{cmd}: {}",
            r.summary
        );
    }
    let mut g = f.gate(GateConfig::default());
    let d = g
        .evaluate(
            &state(node_criteria(), &[]),
            true,
            &CancellationToken::new(),
        )
        .await;
    let GateDecision::Stop { status, reason, .. } = d else {
        panic!("{d:?}")
    };
    assert_eq!(
        (status, reason.as_str()),
        (TaskStatus::Blocked, "verification_blocked")
    );
}

#[tokio::test]
async fn env_and_port_problems_are_blocked_and_zero_tests_unverified() {
    let f = fx(&[]);
    let c = CancellationToken::new();
    let run = |cmd: &'static str| {
        let k = check(&f, "k1", cmd, 5000);
        let r = f.runner();
        let c = c.clone();
        async move { r.run_one(&k, &c).await }
    };
    assert_eq!(
        run("echo 'listen EADDRINUSE :::3000' >&2; exit 1")
            .await
            .status,
        VerificationStatus::Blocked
    );
    assert_eq!(
        run("echo 'API_KEY environment variable is not set'; exit 1")
            .await
            .status,
        VerificationStatus::Blocked
    );
    assert_eq!(
        run("echo 'assertion failed'; exit 1").await.status,
        VerificationStatus::Failed
    );
    assert_eq!(
        run("echo 'Ran 0 tests in 0.000s'").await.status,
        VerificationStatus::Unverified
    );
}

#[tokio::test]
async fn cwd_outside_workspace_is_refused() {
    let f = fx(&[]);
    let mut k = check(&f, "k1", "true", 5000);
    k.cwd = "/".into();
    let r = f.runner().run_one(&k, &CancellationToken::new()).await;
    assert_eq!(r.status, VerificationStatus::Blocked);
}

#[tokio::test]
async fn qa_task_skips_gate_and_cancel_stops() {
    let f = fx(&[]);
    let mut g = f.gate(GateConfig::default());
    let st = TaskState {
        task_id: "t".into(),
        ..Default::default()
    };
    assert_eq!(
        g.evaluate(&st, true, &CancellationToken::new()).await,
        GateDecision::Complete(None)
    );

    let cancel = CancellationToken::new();
    cancel.cancel();
    let d = g.evaluate(&state(vec![], &[]), true, &cancel).await;
    assert!(matches!(
        d,
        GateDecision::Stop {
            status: TaskStatus::Cancelled,
            ..
        }
    ));
}

#[tokio::test]
async fn regression_is_reported_and_report_roundtrips() {
    let f = fx(&[]);
    let mut g = f.gate(GateConfig::default());
    let c = CancellationToken::new();
    let flag = f.ws.cwd.join("flag");
    let cmd = "test -e flag".to_string(); // relative: a `\` path would be eaten by the shell on Windows
    // attempt 1: passes (flag exists) -> but add a second failing criterion to continue
    fs::write(&flag, "").unwrap();
    let two = vec![
        Criterion::user("c1", "x", Some(&cmd)),
        Criterion::user("c2", "y", Some("exit 1")),
    ];
    assert!(matches!(
        g.evaluate(&state(two.clone(), &["a"]), true, &c).await,
        GateDecision::Continue { .. }
    ));
    fs::remove_file(&flag).unwrap();
    let GateDecision::Continue { report, .. } = g.evaluate(&state(two, &["b"]), true, &c).await
    else {
        panic!("expected Continue")
    };
    assert!(
        report
            .limitations
            .iter()
            .any(|l| l.starts_with("regression:"))
    );

    let json = serde_json::to_string(&*report).unwrap();
    let back: VerificationReport = serde_json::from_str(&json).unwrap();
    assert_eq!(*report, back);
    let Event::VerificationCompleted {
        report_id,
        status,
        attempt,
        ..
    } = report.to_event()
    else {
        panic!()
    };
    assert_eq!(
        (report_id.as_str(), status.as_str(), attempt),
        ("vr-t1-2", "failed", 2)
    );
}

#[test]
fn aggregate_rules() {
    use VerificationStatus::*;
    let mk = |id: &str, st, crit: &str| CheckResult {
        id: id.into(),
        kind: CheckKind::Custom,
        command: Some(id.into()),
        cwd: Path::new("/").into(),
        declared_by: Declarer::User,
        started_at: String::new(),
        finished_at: String::new(),
        duration_ms: 0,
        exit_code: None,
        timed_out: false,
        status: st,
        blocked_reason: None,
        summary: String::new(),
        failure_fingerprint: None,
        criteria_ids: vec![crit.into()],
        artifacts: vec![],
    };
    let p = InferredOnlyPolicy::CapUnverified;
    let mut c = vec![Criterion::user("c1", "", None)];
    assert_eq!(aggregate(&mut c, &[mk("k1", Blocked, "c1")], p).0, Blocked);
    assert_eq!(
        aggregate(&mut c, &[mk("k1", Unverified, "c1")], p).0,
        Unverified
    );
    assert_eq!(aggregate(&mut c, &[], p).0, Unverified);
    assert_eq!(aggregate(&mut c, &[mk("k1", Passed, "c1")], p).0, Passed);
    // a failed inferred check fails the report even when the user criterion passed
    assert_eq!(
        aggregate(&mut c, &[mk("k1", Passed, "c1"), mk("k2", Failed, "")], p).0,
        Failed
    );
    // passed + blocked on the same criterion: best evidence is passed
    assert_eq!(
        aggregate(
            &mut c,
            &[mk("k1", Passed, "c1"), mk("k2", Blocked, "c1")],
            p
        )
        .0,
        Passed
    );
}

struct Policy(Vec<(&'static str, CheckAuth)>);

impl CheckAuthorizer for Policy {
    fn authorize<'a>(
        &'a self,
        c: &'a Check,
        _: &'a CancellationToken,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = CheckAuth> + Send + 'a>> {
        let cmd = c.command.as_deref().unwrap_or("");
        let a = self
            .0
            .iter()
            .find(|(p, _)| cmd.starts_with(p))
            .map_or(CheckAuth::Allow, |(_, a)| a.clone());
        Box::pin(async move { a })
    }
}

#[tokio::test]
async fn authorizer_blocks_a_check_without_running_or_failing_it() {
    let f = fx(&[]);
    let r = f.runner();
    let checks = [
        check(&f, "ok", "echo fine", 10_000),
        check(&f, "ask", "touch ran-anyway", 10_000),
        check(&f, "deny", "echo no", 10_000),
    ];
    let policy = Policy(vec![
        (
            "touch",
            CheckAuth::Blocked(format!("{NEEDS_APPROVAL}Bash(touch:*)")),
        ),
        (
            "echo no",
            CheckAuth::Blocked("denied by rule Bash(echo no)".into()),
        ),
    ]);
    let out = r
        .run_with(&checks, &CancellationToken::new(), Some(&policy))
        .await;
    let st: Vec<_> = out.iter().map(|k| k.status).collect();
    assert_eq!(
        st,
        [
            VerificationStatus::Passed,
            VerificationStatus::Blocked,
            VerificationStatus::Blocked
        ]
    );
    assert_eq!(
        out[1].blocked_reason.as_deref(),
        Some("needs_approval:Bash(touch:*)")
    );
    assert!(out[1].exit_code.is_none() && !f.ws.cwd.join("ran-anyway").exists());
    assert!(
        out[2]
            .blocked_reason
            .as_deref()
            .unwrap()
            .starts_with("denied by rule")
    );
}

#[tokio::test]
async fn gate_with_a_blocked_user_check_is_blocked_not_failed() {
    let f = fx(&[]);
    let mut g = f.gate(GateConfig::default());
    let policy = Policy(vec![(
        "echo",
        CheckAuth::Blocked(format!("{NEEDS_APPROVAL}Bash(echo:*)")),
    )]);
    let crit = vec![Criterion::user("c1", "says hi", Some("echo hi"))];
    let d = g
        .evaluate_with(
            &state(crit, &[]),
            true,
            &CancellationToken::new(),
            Some(&policy),
        )
        .await;
    let GateDecision::Stop {
        status,
        report: Some(r),
        ..
    } = d
    else {
        panic!("{d:?}")
    };
    assert_eq!(status, TaskStatus::Blocked);
    assert_eq!(r.checks[0].status, VerificationStatus::Blocked);
}
