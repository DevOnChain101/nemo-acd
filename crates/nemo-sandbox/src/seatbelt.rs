//! macOS Seatbelt via `/usr/bin/sandbox-exec`.
//!
//! `sandbox-exec` is deprecated by Apple with no replacement; it still works but may break
//! in any macOS release, so this is best-effort (reported as `Weak`). Known leaks: file
//! metadata (stat/existence) is readable everywhere, and `mach-lookup` is limited to the
//! [`MACH_SERVICES`] allowlist (what sh, git, node, python and cargo need; extend it when a
//! tool fails with a `mach-lookup` denial in the system log). Reported as `Weak`: defence in
//! depth, NOT a security boundary.
//!
//! Injection safety: user paths never appear in the profile text. They are passed as
//! `-D NAME=value` argv entries and referenced with `(param "NAME")`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::*;

const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";
const SYSTEM_READ: &[&str] = &[
    "/usr",
    "/bin",
    "/sbin",
    "/System",
    "/Library",
    "/private/etc",
    "/opt/homebrew",
    "/dev",
    "/private/var/db",
];

/// Mach services granted to the sandboxed process. Everything else is denied.
const MACH_SERVICES: &[&str] = &[
    // getpwuid/getgrgid (python pwd, cargo/rustup home dir, ls -l, ssh)
    "com.apple.system.opendirectoryd.libinfo",
    // certificate verification for TLS (only useful with NetworkPolicy::Allow)
    "com.apple.trustd",
    "com.apple.trustd.agent",
];

pub struct SeatbeltBackend;

impl SeatbeltBackend {
    /// Some only if `sandbox-exec` exists AND works here (probed; it fails when this process is
    /// already inside a sandbox that forbids nesting).
    pub fn detect() -> Option<Self> {
        Path::new(SANDBOX_EXEC).is_file().then_some(())?;
        std::process::Command::new(SANDBOX_EXEC)
            .args(["-p", "(version 1)(allow default)", "/usr/bin/true"])
            .env_clear()
            .output()
            .is_ok_and(|o| o.status.success())
            .then_some(Self)
    }
}

fn canon(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// Returns the SBPL text and the `-D` params it references.
pub fn seatbelt_profile(
    policy: &SandboxPolicy,
    exec_literal: Option<&Path>,
) -> (String, Vec<(String, PathBuf)>) {
    let mut params: Vec<(String, PathBuf)> = vec![];
    let mut group = |prefix: &str, paths: Vec<PathBuf>| -> String {
        paths
            .into_iter()
            .map(|p| {
                let name = format!("{prefix}{}", params.len());
                params.push((name.clone(), p));
                format!("(subpath (param \"{name}\"))")
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    let canon_all = |v: &[PathBuf]| v.iter().map(|p| canon(p)).collect::<Vec<_>>();

    let tmp: Vec<PathBuf> = policy.tmp_dir.iter().map(|p| canon(p)).collect();
    let mut reads = canon_all(&policy.readable_roots);
    reads.extend(canon_all(&policy.writable_roots));
    reads.extend(tmp.clone());
    let mut writes = canon_all(&policy.writable_roots);
    writes.extend(tmp); // the real /tmp and /var/folders are NOT granted
    let (r, w, d, ro) = (
        group("R", reads),
        group("W", writes),
        group("D", canon_all(&policy.deny_paths)),
        group("O", canon_all(&policy.readonly_paths)),
    );
    let guards = group("G", canon_all(&policy.parent_guards()));

    let mut s = String::from(
        "(version 1)\n(deny default)\n(allow sysctl-read)\n\
         (allow signal (target self))\n(allow process-info* (target self))\n\
         (allow file-read-metadata)\n(allow file-read* (literal \"/\"))\n",
    );
    s += "(allow mach-lookup";
    for m in MACH_SERVICES {
        s += &format!(" (global-name \"{m}\")");
    }
    s += ")\n";
    for p in SYSTEM_READ {
        s += &format!("(allow file-read* (subpath \"{p}\"))\n");
    }
    s += "(allow file-write* (literal \"/dev/null\") (literal \"/dev/tty\") (literal \"/dev/dtracehelper\"))\n";
    if !r.is_empty() {
        s += &format!("(allow file-read* {r})\n");
    }
    s += &format!("(allow file-write* {w})\n");
    match (policy.allow_exec, exec_literal) {
        (true, _) => s += "(allow process-fork)\n(allow process-exec)\n",
        (false, Some(_)) => s += "(allow process-exec (literal (param \"EXEC\")))\n",
        (false, None) => {}
    }
    if policy.network == NetworkPolicy::Allow {
        s += "(allow network*)\n";
    }
    // Last match wins in SBPL: denies go after every allow.
    if !ro.is_empty() {
        s += &format!("(deny file-write* {ro})\n");
    }
    if !d.is_empty() {
        s += &format!("(deny file-read* file-write* {d})\n");
    }
    // Parent directory entries of protected paths: no rename/remove that would move them away.
    // `(literal ..)` matches the directory itself, not its children, so creating files in it works.
    let guards = guards.replace("(subpath ", "(literal ");
    if !guards.is_empty() {
        s += &format!("(deny file-write* {guards})\n");
    }
    if policy.network != NetworkPolicy::Allow {
        s += "(deny network*)\n"; // explicit, also covers AF_UNIX connect
    }
    if policy.network == NetworkPolicy::Loopback {
        // After the deny (last match wins). Measured on macOS 15: `network-inbound` is needed for
        // listen/accept; a combined `(local ip ..)(remote ip ..)` rule would let remote hosts in.
        s += "(allow network* (remote ip \"localhost:*\"))\n\
              (allow network-bind network-inbound (local ip \"localhost:*\"))\n";
    }
    if let (false, Some(e)) = (policy.allow_exec, exec_literal) {
        params.push(("EXEC".into(), canon(e)));
    }
    (s, params)
}

impl SandboxBackend for SeatbeltBackend {
    fn name(&self) -> &'static str {
        "seatbelt"
    }

    fn capabilities(&self) -> SandboxCapabilities {
        SandboxCapabilities {
            filesystem: Guarantee::Weak,
            network: Guarantee::Weak,
            process_tree: Guarantee::Strong,
            notes: vec![
                "sandbox-exec is deprecated by Apple; best-effort, may break on any macOS update".into(),
                "kernel-enforced, inherited by all descendants; file metadata (stat) stays readable".into(),
                "network Loopback exposes the HOST loopback (databases, Redis, docker TCP) to sandboxed code; a 0.0.0.0 listener is LAN-reachable; AF_UNIX stays denied".into(),
                "per-host network allowlists unsupported; allow_exec=false permits only the initial program (macOS /bin/sh re-execs bash, so it cannot run)".into(),
                "tree kill is the terminal runtime's job (process group)".into(),
            ],
        }
    }

    fn network_guarantee(&self, mode: &NetworkPolicy) -> Guarantee {
        network_guarantee_for(mode, self.capabilities().network, Guarantee::Weak)
    }

    fn wrap(
        &self,
        policy: &SandboxPolicy,
        c: &SandboxedCommand,
    ) -> Result<SandboxedCommand, SandboxError> {
        validate(policy)?;
        let exec = if policy.allow_exec {
            None
        } else {
            Some(resolve_program(c).ok_or_else(|| {
                SandboxError::InvalidPolicy(
                    "allow_exec=false needs a resolvable program path".into(),
                )
            })?)
        };
        let (profile, params) = seatbelt_profile(policy, exec.as_deref());
        let mut args: Vec<OsString> = vec!["-p".into(), profile.into()];
        for (k, v) in params {
            let mut kv = OsString::from(format!("{k}="));
            kv.push(v);
            args.extend(["-D".into(), kv]);
        }
        args.push(c.program.clone());
        args.extend(c.args.iter().cloned());
        Ok(SandboxedCommand {
            program: SANDBOX_EXEC.into(),
            args,
            env: build_env(policy, c),
            cwd: c.cwd.clone(),
            clear_env: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_paths_never_reach_profile_text() {
        let evil =
            PathBuf::from("/tmp/a\")) (allow default) (\n(allow file-write* (subpath \"/\"))é");
        let p = SandboxPolicy {
            writable_roots: vec![evil.clone()],
            deny_paths: vec![evil.clone()],
            ..Default::default()
        };
        let (s, params) = seatbelt_profile(&p, None);
        assert!(!s.contains("allow default") && !s.contains('é'));
        assert!(params.iter().any(|(_, v)| *v == evil));
        assert!(s.trim_end().ends_with(')'));
    }

    #[test]
    fn network_and_exec_rendering() {
        let mut p = SandboxPolicy::default();
        let s = seatbelt_profile(&p, None).0;
        assert!(!s.contains("(allow network*)") && s.contains("(deny network*)"));
        p.network = NetworkPolicy::Allow;
        p.allow_exec = false;
        let (s, params) = seatbelt_profile(&p, Some(Path::new("/bin/sh")));
        assert!(s.contains("(allow network*)") && !s.contains("(deny network*)"));
        assert!(!s.contains("process-fork"));
        assert!(params.iter().any(|(k, _)| k == "EXEC"));
    }

    #[test]
    fn loopback_rules_come_after_the_deny() {
        let p = SandboxPolicy {
            network: NetworkPolicy::Loopback,
            ..Default::default()
        };
        let s = seatbelt_profile(&p, None).0;
        let deny = s.find("(deny network*)").unwrap();
        let allow = s
            .find("(allow network* (remote ip \"localhost:*\"))")
            .unwrap();
        assert!(deny < allow && !s.contains("(allow network*)"));
        assert!(s.contains("(allow network-bind network-inbound (local ip \"localhost:*\"))"));
        let b = SeatbeltBackend;
        assert_eq!(
            b.network_guarantee(&NetworkPolicy::Loopback),
            Guarantee::Weak
        );
    }
}
