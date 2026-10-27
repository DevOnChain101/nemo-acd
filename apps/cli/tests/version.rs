use std::process::Command;

#[test]
fn version_exits_zero() {
    let out = Command::new(env!("CARGO_BIN_EXE_nemo"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(out.status.success());
    let s = String::from_utf8(out.stdout).unwrap();
    assert_eq!(s.trim(), format!("nemo {}", env!("CARGO_PKG_VERSION")));
}
