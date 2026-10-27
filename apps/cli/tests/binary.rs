//! Binary-level tests that need no model: flags, exit codes, secrets hygiene.
use std::path::Path;
use std::process::{Command, Output};

const KEY: &str = "sk-planted-SECRETKEY0123456789abcdef";

fn nemo(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nemo"))
        .args(args)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", home)
        .env("NEMO_HOME", home.join("nemo"))
        .env("NEMO_API_KEY", KEY)
        .env("HTTPS_PROXY", "http://user:proxysecret@proxy.invalid:3128")
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap()
}

fn text(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

#[test]
fn run_without_cwd_is_a_usage_error() {
    let h = tempfile::tempdir().unwrap();
    let o = nemo(h.path(), &["run", "--prompt", "x"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(o.stdout.is_empty());
    assert!(text(&o.stderr).contains("--cwd"), "{}", text(&o.stderr));
}

#[test]
fn run_with_bad_cwd_or_no_prompt_exits_2_without_events() {
    let h = tempfile::tempdir().unwrap();
    let missing = h.path().join("nope");
    let o = nemo(
        h.path(),
        &["run", "--cwd", missing.to_str().unwrap(), "--prompt", "x"],
    );
    assert_eq!(o.status.code(), Some(2));
    assert!(o.stdout.is_empty());
    let o = nemo(h.path(), &["run", "--cwd", h.path().to_str().unwrap()]); // stdin is /dev/null
    assert_eq!(o.status.code(), Some(2));
    assert!(o.stdout.is_empty());
}

#[test]
fn run_without_model_exits_2_and_never_prints_the_key() {
    let h = tempfile::tempdir().unwrap();
    let o = nemo(
        h.path(),
        &["run", "--cwd", h.path().to_str().unwrap(), "--prompt", "x"],
    );
    assert_eq!(o.status.code(), Some(2));
    assert!(o.stdout.is_empty());
    assert!(!text(&o.stderr).contains(KEY));
}

#[test]
fn auth_status_masks_the_key() {
    let h = tempfile::tempdir().unwrap();
    let o = nemo(h.path(), &["auth", "status"]);
    assert!(o.status.success());
    let out = text(&o.stdout);
    assert!(!out.contains(KEY) && !text(&o.stderr).contains(KEY));
    assert!(
        out.contains("anthropic: sk-p…cdef (source: env NEMO_API_KEY)"),
        "{out}"
    );
    assert!(out.contains("openai: not configured"), "{out}");
}

#[test]
fn doctor_reports_without_secrets() {
    let h = tempfile::tempdir().unwrap();
    let o = nemo(h.path(), &["doctor"]);
    assert!(o.status.success(), "{}", text(&o.stderr));
    let out = text(&o.stdout);
    for s in [KEY, "proxysecret", "user:"] {
        assert!(!out.contains(s), "leaked {s}");
    }
    for want in [
        "shell:",
        "sandbox for tool commands",
        "default mode: enforce",
        "browser:",
        "git:",
        "NEMO_HOME:",
        "config: ok",
        "HTTPS_PROXY: set",
    ] {
        assert!(out.contains(want), "missing {want}:\n{out}");
    }
}

#[test]
fn doctor_names_the_invalid_config_file() {
    let h = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(h.path().join("nemo")).unwrap();
    std::fs::write(h.path().join("nemo/nemo.json"), "{\"model\": 5}").unwrap();
    let o = nemo(h.path(), &["doctor"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(text(&o.stdout).contains("nemo.json"), "{}", text(&o.stdout));
}

#[test]
fn models_json_comes_from_config_without_network() {
    let h = tempfile::tempdir().unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_nemo"))
        .args(["models", "--json"])
        .env_clear()
        .env("HOME", h.path())
        .env("NEMO_HOME", h.path().join("nemo"))
        .env("NEMO_MODEL", "anthropic/test-model")
        .output()
        .unwrap();
    assert!(o.status.success(), "{}", text(&o.stderr));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["default"], "anthropic/test-model");
    assert!(
        v["models"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["value"] == "anthropic/test-model" && m["source"] == "config")
    );
}

#[cfg(not(feature = "test-provider"))]
#[test]
fn fake_provider_is_refused_without_the_feature() {
    let h = tempfile::tempdir().unwrap();
    let o = nemo(
        h.path(),
        &[
            "run",
            "--cwd",
            h.path().to_str().unwrap(),
            "--prompt",
            "x",
            "--provider",
            "fake:/x.json",
        ],
    );
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn no_subcommand_without_a_terminal_prints_usage_and_exits_2() {
    let h = tempfile::tempdir().unwrap();
    let o = nemo(h.path(), &["--cwd", h.path().to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(2));
    assert!(text(&o.stderr).contains("needs a terminal"));
    assert!(o.stdout.is_empty());
}

#[test]
fn once_renders_one_frame_without_a_terminal() {
    let h = tempfile::tempdir().unwrap();
    let o = nemo(h.path(), &["--once", "--cwd", h.path().to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    assert!(!o.stdout.is_empty());
}

#[test]
fn once_shows_the_startup_warnings() {
    let h = tempfile::tempdir().unwrap();
    let cwd = h.path().to_str().unwrap();
    let o = nemo(
        h.path(),
        &[
            "--once",
            "--cwd",
            cwd,
            "--sandbox",
            "off",
            "--network",
            "allow",
        ],
    );
    assert_eq!(o.status.code(), Some(0), "{}", text(&o.stderr));
    let out = text(&o.stdout);
    assert!(
        out.contains("unsandboxed_by_request: --sandbox off"),
        "{out}"
    );
    assert!(out.contains("--network allow"), "{out}");
}
