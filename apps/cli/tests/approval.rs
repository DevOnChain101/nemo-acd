//! Autonomous command running without an approver: the denial text, `needed_permissions`, the
//! printed hint and `--allow` / `--deny` (feature `test-provider`).
#![cfg(feature = "test-provider")]

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use serde_json::{Value, json};

struct Rig {
    dir: tempfile::TempDir,
}

fn host_confines() -> bool {
    static C: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *C.get_or_init(|| {
        let d = tempfile::tempdir().unwrap();
        let o = Command::new(env!("CARGO_BIN_EXE_nemo"))
            .args(["doctor", "--cwd"])
            .arg(d.path())
            .env("NEMO_HOME", d.path())
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout).contains("enforce_ready: yes")
    })
}

impl Rig {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("ws")).unwrap();
        Rig { dir }
    }
    fn ws(&self) -> PathBuf {
        self.dir.path().join("ws")
    }
    fn run(&self, script: Value, args: &[&str]) -> (Output, Vec<Value>) {
        let s = self.dir.path().join("script.json");
        std::fs::write(&s, script.to_string()).unwrap();
        let mut c = Command::new(env!("CARGO_BIN_EXE_nemo"));
        c.args(["run", "--cwd"])
            .arg(self.ws())
            .arg("--provider")
            .arg(format!("fake:{}", s.display()))
            .args(["--prompt", "do it"])
            .args(args);
        if !host_confines() {
            c.args(["--sandbox", "off"]);
        }
        c.env_clear();
        for k in [
            "PATH",
            "SystemRoot",
            "PATHEXT",
            "ComSpec",
            "USERPROFILE",
            "TEMP",
            "TMP",
        ] {
            if let Some(v) = std::env::var_os(k) {
                c.env(k, v);
            }
        }
        c.env("HOME", self.dir.path())
            .env("NEMO_HOME", self.dir.path().join("nemo"))
            .stdin(Stdio::null());
        let o = c.output().unwrap();
        let ev = String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect();
        (o, ev)
    }
}

fn bash(cmd: &str) -> Value {
    json!([
        {"tools": [{"id": "b1", "name": "Bash", "input": {"command": cmd}}]},
        {"text": "blocked: I need permission to run that"}
    ])
}

fn ended(ev: &[Value]) -> &Value {
    &ev.iter().rev().find(|e| e["type"] == "task.ended").unwrap()["payload"]
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn headless_run_prints_what_permission_is_needed_and_how_to_grant_it() {
    let r = Rig::new();
    let (o, ev) = r.run(bash("cargo test"), &[]);
    let e = stderr(&o);
    assert!(
        e.contains("nemo: blocked: approval needed for Bash(cargo test:*)")
            && e.contains("rerun with --allow 'Bash(cargo test:*)' or in the TUI"),
        "{e}"
    );
    assert_eq!(o.status.code(), Some(4), "{e}");
    let p = ended(&ev);
    assert_eq!(p["status"], "blocked");
    assert_eq!(p["reason"], "approval_unavailable");
    assert_eq!(p["needed_permissions"], json!(["Bash(cargo test:*)"]));
    let done = ev.iter().find(|e| e["type"] == "tool.completed").unwrap();
    let out = done["payload"]["output"].as_str().unwrap();
    assert!(
        out.contains("requires approval; no approver available in this run")
            && out.contains("do not ask the user to run it manually")
            && out.contains("name the permission needed: Bash(cargo test:*)"),
        "{out}"
    );
    // a trail of the refusal, not a silent skip
    assert!(ev.iter().any(
        |e| e["type"] == "tool.approval_resolved" && e["payload"]["decision"] == "auto_denied"
    ));
}

#[test]
fn allow_flag_grants_the_rule_deny_wins_and_protections_stay() {
    let r = Rig::new();
    // without the flag: blocked, nothing created
    let (o, _) = r.run(bash("touch made.txt"), &[]);
    assert_eq!(o.status.code(), Some(4), "{}", stderr(&o));
    assert!(!r.ws().join("made.txt").exists());

    // with it: runs (the task is then unverified: a side effect and no gate checks)
    let (o, ev) = r.run(bash("touch made.txt"), &["--allow", "Bash(touch:*)"]);
    assert!(r.ws().join("made.txt").exists(), "{}", stderr(&o));
    assert!(ended(&ev).get("needed_permissions").is_none());

    // deny beats allow, and it is a policy refusal, not a missing approval
    std::fs::remove_file(r.ws().join("made.txt")).unwrap();
    let (o, ev) = r.run(
        bash("touch made.txt"),
        &["--allow", "Bash(touch:*)", "--deny", "Bash(touch:*)"],
    );
    assert!(!r.ws().join("made.txt").exists());
    assert!(ended(&ev).get("needed_permissions").is_none());
    assert!(!stderr(&o).contains("approval needed"), "{}", stderr(&o));

    // an allow rule cannot lift the protected-path check
    let (_, _) = r.run(bash("echo SECRET=1 > .env"), &["--allow", "Bash(echo:*)"]);
    assert!(!r.ws().join(".env").exists(), "protected path written");
}

#[test]
fn malformed_rule_flags_are_usage_errors() {
    let r = Rig::new();
    for bad in ["", "Bash(cargo test:*", "(x)", "a b"] {
        let (o, ev) = r.run(json!([{"text": "x"}]), &["--allow", bad]);
        assert_eq!(o.status.code(), Some(2), "{bad:?}: {}", stderr(&o));
        assert!(ev.is_empty(), "no session started for {bad:?}");
        assert!(stderr(&o).contains("--allow"), "{}", stderr(&o));
    }
}

#[test]
fn gate_check_that_needs_approval_blocks_the_task_with_a_hint() {
    let r = Rig::new();
    let crit = r.dir.path().join("criteria.json");
    std::fs::write(
        &crit,
        json!([{"id": "c1", "text": "make check passes", "check": "make check"}]).to_string(),
    )
    .unwrap();
    let script = json!([
        {"tools": [{"id": "w1", "name": "Write", "input": {"file_path": "a.txt", "content": "x"}}]},
        {"text": "done"},
        {"text": "done again"}
    ]);
    let (o, ev) = r.run(
        script,
        &[
            "--permission-mode",
            "acceptEdits",
            "--criteria-file",
            crit.to_str().unwrap(),
        ],
    );
    let e = stderr(&o);
    assert_eq!(o.status.code(), Some(4), "{e}");
    let p = ended(&ev);
    assert_eq!(
        (p["status"].as_str(), p["reason"].as_str()),
        (Some("blocked"), Some("approval_unavailable"))
    );
    assert_eq!(p["needed_permissions"], json!(["Bash(make:*)"]));
    assert!(e.contains("--allow 'Bash(make:*)'"), "{e}");
    let v = ev
        .iter()
        .find(|e| e["type"] == "verification.completed")
        .unwrap();
    assert!(
        v["payload"]["summary"]
            .as_str()
            .unwrap()
            .contains("1 blocked"),
        "{v}"
    );
}
