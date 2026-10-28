//! Check planning (verification.md §4.2). Reads files only; never runs anything.

use crate::types::*;
use std::fs;
use std::path::Path;

fn chk(kind: CheckKind, cmd: &str, cwd: &Path) -> Check {
    Check {
        id: String::new(),
        kind,
        command: Some(cmd.into()),
        cwd: cwd.into(),
        timeout_ms: DEFAULT_TIMEOUT_MS,
        declared_by: Declarer::Harness,
        criteria_ids: vec![],
    }
}

/// Project commands discovered in `dir` (shortcut: top level only, no monorepo walk).
pub fn detect_checks(dir: &Path) -> Vec<Check> {
    let mut out = vec![];
    let has = |f: &str| dir.join(f).is_file();
    let read = |f: &str| fs::read_to_string(dir.join(f)).unwrap_or_default();

    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&read("package.json")) {
        let pm = if has("pnpm-lock.yaml") {
            "pnpm"
        } else if has("yarn.lock") {
            "yarn"
        } else {
            "npm"
        };
        for (script, kind) in [
            ("typecheck", CheckKind::Typecheck),
            ("lint", CheckKind::Lint),
            ("build", CheckKind::Build),
            ("test", CheckKind::Unit),
        ] {
            let Some(body) = v["scripts"][script].as_str() else {
                continue;
            };
            if script == "test" && body.contains("no test specified") {
                continue; // npm init placeholder
            }
            out.push(chk(kind, &format!("{pm} run {script}"), dir));
        }
    }
    if has("Cargo.toml") {
        out.push(chk(CheckKind::Syntax, "cargo fmt --check", dir));
        out.push(chk(
            CheckKind::Lint,
            "cargo clippy --all-targets -- -D warnings",
            dir,
        ));
        out.push(chk(CheckKind::Unit, "cargo test", dir));
    }
    let py = if cfg!(windows) { "python" } else { "python3" };
    let pytest_cfg = has("pytest.ini") || read("pyproject.toml").contains("pytest");
    let has_py_tests = [dir.to_path_buf(), dir.join("tests"), dir.join("test")]
        .iter()
        .filter_map(|d| fs::read_dir(d).ok())
        .flatten()
        .flatten()
        .any(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.starts_with("test") && n.ends_with(".py")
        });
    if pytest_cfg {
        out.push(chk(CheckKind::Unit, &format!("{py} -m pytest"), dir));
    } else if has_py_tests {
        out.push(chk(
            CheckKind::Unit,
            &format!("{py} -m unittest discover"),
            dir,
        ));
    }
    if read("Makefile")
        .lines()
        .any(|l| l.starts_with("test:") || l.starts_with("test :"))
    {
        out.push(chk(CheckKind::Unit, "make test", dir));
    }
    if has("go.mod") {
        out.push(chk(CheckKind::Unit, "go test ./...", dir));
    }
    out
}

/// A recognised verification invocation: one plain command (no shell syntax) that only builds,
/// lints or tests. The harness may run it unasked when the harness itself inferred it
/// (docs/specs/verification.md §4.3). Shortcut: token match on the leading words, so `make` takes
/// no variable overrides and flags that run other programs are refused; widen on demand.
pub fn is_verification_command(cmd: &str) -> bool {
    const PLAIN: &str = " _.-/=:,@+";
    if cmd.is_empty()
        || !cmd
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || PLAIN.contains(c))
    {
        return false;
    }
    let t: Vec<&str> = cmd.split_whitespace().collect();
    if t.iter().any(|a| {
        ["--fix", "--config", "-exec", "-toolexec", "-vettool"]
            .iter()
            .any(|bad| a == bad || a.starts_with(&format!("{bad}=")))
    }) {
        return false;
    }
    match t.as_slice() {
        ["cargo", "fmt", rest @ ..] => rest.contains(&"--check"),
        ["cargo", "clippy" | "test" | "check", ..] => true,
        [
            "npm" | "pnpm" | "yarn",
            "run",
            "test" | "lint" | "typecheck" | "build",
            ..,
        ] => true,
        ["node", "--test", ..] | ["pytest", ..] => true,
        ["python" | "python3", "-m", "pytest" | "unittest", ..] => true,
        ["go", "test" | "vet", ..] => true,
        ["make", "test" | "lint" | "check"] => true,
        ["tsc", rest @ ..] => rest.contains(&"--noEmit"),
        _ => false,
    }
}

/// Whether `changed` (workspace-relative paths) can affect what the inferred check `cmd` exercises.
/// A change set of only unrelated files (a `.txt`, a README) makes the check noise.
fn relevant(cmd: &str, changed: &[String]) -> bool {
    let exts: &[&str] = match cmd.split_whitespace().next() {
        Some("cargo") => &["rs", "toml", "lock"],
        Some("npm" | "pnpm" | "yarn") => &[
            "js", "jsx", "ts", "tsx", "mjs", "cjs", "json", "vue", "svelte",
        ],
        Some("python" | "python3") => &["py", "toml", "ini", "cfg"],
        Some("go") => &["go", "mod", "sum"],
        Some("make") => {
            return changed
                .iter()
                .any(|f| !f.ends_with(".md") && !f.ends_with(".txt"));
        }
        _ => return true,
    };
    changed.iter().any(|f| {
        Path::new(f)
            .extension()
            .is_some_and(|e| exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
    })
}

/// Plan order: user-declared criterion checks, model-registered checks, inferred checks.
/// Inferred checks duplicating an earlier command are dropped; ids `k1..` are assigned to
/// checks that have none. With a non-empty `changed`, inferred checks unrelated to it are dropped.
pub fn plan_checks(
    criteria: &[Criterion],
    model_checks: &[Check],
    dir: &Path,
    changed: &[String],
) -> Vec<Check> {
    let mut out: Vec<Check> = vec![];
    for c in criteria
        .iter()
        .filter(|c| c.source == CriterionSource::User)
    {
        if let Some(cmd) = &c.check {
            let mut k = chk(CheckKind::Custom, cmd, dir);
            k.declared_by = Declarer::User;
            k.criteria_ids = vec![c.id.clone()];
            out.push(k);
        }
    }
    out.extend(model_checks.iter().cloned().map(|mut k| {
        k.declared_by = Declarer::Model;
        k
    }));
    for k in detect_checks(dir) {
        let rel = changed.is_empty() || k.command.as_deref().is_none_or(|c| relevant(c, changed));
        if rel && !out.iter().any(|o| o.command == k.command) {
            out.push(k);
        }
    }
    for (i, k) in out.iter_mut().enumerate() {
        if k.id.is_empty() {
            k.id = format!("k{}", i + 1);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_without_running_and_maps_criteria() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        fs::write(
            p.join("package.json"),
            r#"{"scripts":{"test":"node --test","lint":"x","build":"y","start":"z"}}"#,
        )
        .unwrap();
        fs::write(p.join("Cargo.toml"), "").unwrap();
        fs::write(p.join("go.mod"), "").unwrap();
        fs::write(p.join("Makefile"), "all:\n\ttrue\ntest:\n\ttrue\n").unwrap();
        fs::write(p.join("test_a.py"), "").unwrap();
        let cmds: Vec<_> = detect_checks(p)
            .into_iter()
            .map(|c| c.command.unwrap())
            .collect();
        for want in [
            "npm run test",
            "npm run lint",
            "npm run build",
            "cargo test",
            "cargo fmt --check",
            "go test ./...",
            "make test",
        ] {
            assert!(cmds.iter().any(|c| c == want), "{want} in {cmds:?}");
        }
        assert!(cmds.iter().any(|c| c.ends_with("-m unittest discover")));
        assert!(!cmds.iter().any(|c| c.contains("start")));

        let crit = [Criterion::user("c1", "works", Some("cargo test"))];
        let plan = plan_checks(&crit, &[], p, &[]);
        assert_eq!(plan[0].declared_by, Declarer::User);
        assert_eq!(plan[0].criteria_ids, ["c1"]);
        assert_eq!(
            plan.iter()
                .filter(|c| c.command.as_deref() == Some("cargo test"))
                .count(),
            1
        );
    }

    #[test]
    fn recognises_only_plain_verification_commands() {
        for ok in [
            "cargo fmt --check",
            "cargo clippy --all-targets -- -D warnings",
            "cargo test -p x -- --nocapture",
            "cargo check",
            "npm run test",
            "pnpm run lint",
            "yarn run typecheck",
            "npm run build",
            "node --test",
            "pytest -q tests/",
            "python3 -m pytest",
            "python -m unittest discover",
            "go test ./...",
            "go vet ./...",
            "make test",
            "tsc --noEmit",
        ] {
            assert!(is_verification_command(ok), "{ok}");
        }
        for bad in [
            "",
            "cargo fmt",
            "cargo fix",
            "cargo test && rm -rf x",
            "cargo test; id",
            "cargo test $(id)",
            "cargo test > out",
            "cargo clippy --fix",
            "cargo test --config build.rustc-wrapper=sh",
            "go test -exec sh ./...",
            "npm run deploy",
            "npm test",
            "make test CC=sh",
            "make install",
            "tsc",
            "python -m http.server",
            "curl x",
        ] {
            assert!(!is_verification_command(bad), "{bad}");
        }
    }

    #[test]
    fn every_detected_check_is_recognised() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("Cargo.toml"), "").unwrap();
        fs::write(d.path().join("go.mod"), "").unwrap();
        fs::write(d.path().join("Makefile"), "test:\n").unwrap();
        fs::write(d.path().join("pytest.ini"), "").unwrap();
        fs::write(
            d.path().join("package.json"),
            r#"{"scripts":{"typecheck":"x","lint":"x","build":"x","test":"x"}}"#,
        )
        .unwrap();
        let all = detect_checks(d.path());
        assert!(all.len() >= 9);
        for c in all {
            let cmd = c.command.unwrap();
            assert!(is_verification_command(&cmd), "{cmd}");
        }
    }

    #[test]
    fn unrelated_changes_drop_inferred_checks() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("Cargo.toml"), "").unwrap();
        let cmds = |ch: &[&str]| {
            let ch: Vec<String> = ch.iter().map(|s| s.to_string()).collect();
            plan_checks(&[], &[], d.path(), &ch).len()
        };
        assert_eq!(cmds(&["hello.txt"]), 0);
        assert_eq!(cmds(&["src/lib.rs"]), 3);
        assert_eq!(cmds(&[]), 3);
    }

    #[test]
    fn npm_placeholder_test_is_skipped() {
        let d = tempfile::tempdir().unwrap();
        fs::write(
            d.path().join("package.json"),
            r#"{"scripts":{"test":"echo \"Error: no test specified\" && exit 1"}}"#,
        )
        .unwrap();
        assert!(detect_checks(d.path()).is_empty());
    }
}
