//! Real Landlock enforcement through the helper binary. Linux only.
//! NOT RUN on the macOS dev machine that wrote it (compiled via --target x86_64-unknown-linux-gnu).
#![cfg(target_os = "linux")]

use std::path::Path;
use std::process::{Command, Output};

use synthesis_sandbox::{
    LinuxLandlockBackend, NetworkPolicy, SandboxBackend, SandboxPolicy, SandboxedCommand,
    landlock_abi,
};

const HELPER: &str = env!("CARGO_BIN_EXE_synthesis-sandbox-exec");

fn tmp() -> tempfile::TempDir {
    tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap()
}

/// None (test skipped) when the kernel has no Landlock.
fn sh(policy: &SandboxPolicy, cwd: &Path, script: &str) -> Option<Output> {
    let abi = landlock_abi()?;
    let cmd = SandboxedCommand {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), script.into()],
        cwd: Some(cwd.to_path_buf()),
        ..Default::default()
    };
    let w = LinuxLandlockBackend::new(HELPER.into(), abi)
        .wrap(policy, &cmd)
        .unwrap();
    Some(
        Command::new(&w.program)
            .args(&w.args)
            .env_clear()
            .envs(w.env.iter().cloned())
            .current_dir(cwd)
            .output()
            .unwrap(),
    )
}

fn pol(w: &Path) -> SandboxPolicy {
    SandboxPolicy {
        writable_roots: vec![w.canonicalize().unwrap()],
        network: NetworkPolicy::Allow,
        ..Default::default()
    }
}

#[test]
fn write_inside_ok_outside_denied() {
    let (a, b) = (tmp(), tmp());
    let p = pol(a.path());
    let Some(ok) = sh(&p, a.path(), "echo hi > in.txt") else {
        return;
    };
    assert!(
        ok.status.success(),
        "{}",
        String::from_utf8_lossy(&ok.stderr)
    );
    let out = b.path().join("out.txt");
    let bad = sh(&p, a.path(), &format!("echo hi > '{}'", out.display())).unwrap();
    assert!(!bad.status.success() && !out.exists());
}

#[test]
fn tcp_connect_denied_when_network_denied() {
    let a = tmp();
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let p = SandboxPolicy {
        network: NetworkPolicy::Deny,
        ..pol(a.path())
    };
    // bash's /dev/tcp; the helper refuses to run at all when the kernel lacks ABI 4.
    let script = format!("exec 3<>/dev/tcp/127.0.0.1/{port}");
    if landlock_abi().is_some_and(|v| v >= 4) {
        let r = sh(&p, a.path(), &format!("bash -c '{script}'")).unwrap();
        assert!(!r.status.success());
    }
}

#[test]
fn helper_fails_closed_on_bad_policy() {
    let o = Command::new(HELPER)
        .args(["{\"version\":9}", "--", "/bin/true"])
        .output()
        .unwrap();
    assert_eq!(o.status.code(), Some(126));
}
