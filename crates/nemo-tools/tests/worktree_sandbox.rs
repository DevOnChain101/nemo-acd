//! Real Seatbelt enforcement of the worktree git pin (security finding #9). macOS only.
#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::process::Command;

use nemo_core::WorkspaceContext;
use nemo_sandbox::{SandboxBackend, SandboxPolicy, SandboxedCommand, SeatbeltBackend};
use nemo_terminal::NetworkPolicy;
use nemo_tools::standard_policy_git;

fn git(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git")
        .current_dir(dir)
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args(args)
        .output()
        .unwrap();
    assert!(o.status.success(), "git {args:?}: {o:?}");
    String::from_utf8(o.stdout).unwrap()
}

fn sh(p: &SandboxPolicy, cwd: &Path, script: &str) -> std::process::Output {
    let cmd = SandboxedCommand {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), script.into()],
        cwd: Some(cwd.to_path_buf()),
        ..Default::default()
    };
    let w = SeatbeltBackend::detect().unwrap().wrap(p, &cmd).unwrap();
    Command::new(&w.program)
        .args(&w.args)
        .env_clear()
        .envs(w.env.iter().cloned())
        .current_dir(cwd)
        .output()
        .unwrap()
}

struct Fixture {
    _t: tempfile::TempDir,
    main: PathBuf,
    wt: PathBuf,
    git_dir: PathBuf,
    ws: WorkspaceContext,
}

fn fixture() -> Fixture {
    let t = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let base = t.path().canonicalize().unwrap();
    let main = base.join("main");
    std::fs::create_dir(&main).unwrap();
    git(&main, &["init", "-q"]);
    std::fs::write(main.join("f.txt"), "one\n").unwrap();
    git(&main, &["add", "."]);
    git(&main, &["commit", "-qm", "init"]);
    let wt = base.join("wt");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "nemo/x",
            wt.to_str().unwrap(),
        ],
    );
    let git_dir = PathBuf::from(git(&wt, &["rev-parse", "--absolute-git-dir"]).trim());
    let ws = WorkspaceContext::open("w", &wt, &wt, &[], base.join("art"), "pg").unwrap();
    Fixture {
        _t: t,
        main,
        wt,
        git_dir,
        ws,
    }
}

#[test]
fn sandbox_cannot_redirect_the_worktree_git_pin() {
    // A git that does not run even outside the sandbox is the environment's problem, not the policy's.
    if !Command::new("/bin/sh")
        .args(["-c", "git --version"])
        .env_clear()
        .output()
        .is_ok_and(|o| o.status.success())
    {
        eprintln!("SKIP: git does not run outside the sandbox either");
        return;
    }
    let f = fixture();
    let p = standard_policy_git(&f.ws, NetworkPolicy::Deny, Some(&f.git_dir)).unwrap();
    let ok = |s: &str| {
        let o = sh(&p, &f.wt, s);
        assert!(
            o.status.success(),
            "{s}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        String::from_utf8_lossy(&o.stdout).into_owned()
    };
    ok("echo hi > a.txt && echo two >> f.txt");
    assert!(ok("git status --porcelain").contains("a.txt"));
    assert!(ok("git diff").contains("+two"));
    let gd = f.git_dir.display();
    let main_git = f.main.join(".git");
    let pointer = std::fs::read_to_string(f.wt.join(".git")).unwrap();
    for bad in [
        "echo 'gitdir: /tmp/evil' > .git".to_string(),
        "echo x >> .git".into(),
        "rm .git".into(),
        "mv .git .git.bak".into(),
        format!("echo ref: refs/heads/evil > '{gd}/HEAD'"),
        format!("echo x > '{gd}/config'"),
        format!("mv '{gd}' '{gd}.bak'"),
        format!("echo x > '{}/config'", main_git.display()),
    ] {
        assert!(!sh(&p, &f.wt, &bad).status.success(), "{bad}");
    }
    assert_eq!(std::fs::read_to_string(f.wt.join(".git")).unwrap(), pointer);
    assert!(f.git_dir.join("HEAD").is_file() && !f.wt.join(".git.bak").exists());
    assert_eq!(
        git(&f.wt, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
        "nemo/x"
    );
}

#[test]
fn main_git_under_a_writable_root_is_an_error() {
    let f = fixture();
    let mut ws = f.ws.clone();
    ws.additional_dirs.push(f.main.clone());
    let e = standard_policy_git(&ws, NetworkPolicy::Deny, Some(&f.git_dir)).unwrap_err();
    assert!(e.contains("must not be writable"), "{e}");
    let bad = f.main.join("nope");
    assert!(standard_policy_git(&f.ws, NetworkPolicy::Deny, Some(&bad)).is_err());
}
