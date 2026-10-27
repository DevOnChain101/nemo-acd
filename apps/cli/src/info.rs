//! `auth status`, `models`, `doctor`. Output is for humans (or `--json`); never prints a secret.

use std::ffi::OsString;
use std::path::Path;

use clap::Args;
use nemo_config::{Config, Env, ProjectTrust, list_models, resolve_credential};
use nemo_harness::sanitize_for_display as san;
use nemo_providers::{NetSettings, ProviderSpec, RetryPolicy};
use nemo_sandbox::SandboxBackend;
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::{homes, load_config};

pub fn auth_status(env: &Env, cwd: Option<&Path>) -> Result<u8, String> {
    let h = homes(env)?;
    let cfg = load_config(env, &h, cwd, ProjectTrust::Untrusted)?;
    let flat = env.get_nonempty("NEMO_PROVIDER").unwrap_or("anthropic");
    out!("NEMO_HOME: {}", h.nemo.display());
    out!("flat NEMO_API_KEY / NEMO_AUTH_TOKEN apply to provider: {flat}");
    for (id, p) in &cfg.providers {
        match resolve_credential(id, Some(p), &h.nemo, env) {
            Some(c) => out!("{id}: {} (source: {})", c.masked(), c.source),
            None if p.is_no_auth() => out!("{id}: no authentication required"),
            None => out!("{id}: not configured"),
        }
    }
    Ok(0)
}

#[derive(Args)]
pub struct ModelsArgs {
    #[command(flatten)]
    pub cwd: crate::CwdArg,
    #[arg(long)]
    json: bool,
    /// Ask each provider with a credential for its model list (network).
    #[arg(long)]
    refresh: bool,
}

pub fn models(env: &Env, a: &ModelsArgs) -> Result<u8, String> {
    let h = homes(env)?;
    let cfg = load_config(env, &h, a.cwd.cwd.as_deref(), ProjectTrust::Untrusted)?;
    let mut rows: Vec<(String, String, String, &str)> = list_models(&cfg, None)
        .into_iter()
        .map(|m| (m.value, m.provider, m.model, "config"))
        .collect();
    if a.refresh {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        for (id, p) in cfg
            .providers
            .iter()
            .filter(|(_, p)| p.discover != Some(false))
        {
            let cred = resolve_credential(id, Some(p), &h.nemo, env);
            let found = ProviderSpec::from_config(id, p, cred.as_ref())
                .and_then(|s| s.connect(env, &NetSettings::default(), RetryPolicy::default()))
                .map(|prov| {
                    rt.block_on(async move { prov.list_models(&CancellationToken::new()).await })
                });
            match found {
                Ok(Ok(list)) => {
                    for m in list.iter().filter_map(|m| m.id.as_str()) {
                        let value = format!("{id}/{m}");
                        if !rows.iter().any(|r| r.0 == value) {
                            rows.push((value, id.clone(), m.to_string(), "discovered"));
                        }
                    }
                }
                Ok(Err(e)) | Err(e) => {
                    eprintln!("nemo: {id}: model discovery failed: {}", e.message())
                }
            }
        }
    }
    if a.json {
        let models: Vec<_> = rows
            .iter()
            .map(|(v, p, m, s)| json!({"value": v, "provider": p, "model": m, "source": s}))
            .collect();
        out!("{}", json!({"default": cfg.model, "models": models}));
    } else {
        for (v, ..) in &rows {
            out!("{v}");
        }
    }
    Ok(0)
}

fn guarantees(b: &dyn SandboxBackend) -> String {
    let c = b.capabilities();
    format!(
        "{}: filesystem={:?} network={:?} process_tree={:?}\n      {}",
        b.name(),
        c.filesystem,
        c.network,
        c.process_tree,
        c.notes.join("\n      ")
    )
}

/// Per-mode guarantee of one backend, e.g. `deny=Weak loopback=None allow=Strong`.
fn network_support(b: &dyn SandboxBackend) -> String {
    use nemo_sandbox::NetworkPolicy::{Allow, Deny, Loopback};
    [("deny", Deny), ("loopback", Loopback), ("allow", Allow)]
        .iter()
        .map(|(n, m)| format!("{n}={:?}", b.network_guarantee(m)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn git_version(env: &Env) -> Option<String> {
    let path = env.get_nonempty("PATH")?;
    let exe = std::env::split_paths(path)
        .flat_map(|d| [d.join("git"), d.join("git.exe")])
        .find(|p| p.is_file())?;
    let out = std::process::Command::new(exe)
        .arg("--version")
        .env_clear()
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

pub fn doctor(env: &Env, cwd: Option<&Path>) -> Result<u8, String> {
    let mut code = 0;
    out!(
        "nemo {} on {} {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    let get = |k: &str| env.get(k).map(OsString::from);
    let sh = nemo_terminal::resolve_shell(cfg!(windows), &get, &|p: &Path| p.exists());
    out!(
        "shell: {} kind={:?} is_posix={}{}",
        sh.program.display(),
        sh.kind,
        sh.is_posix(),
        sh.note.map(|n| format!(" ({n})")).unwrap_or_default()
    );

    let best = nemo_sandbox::detect();
    let confines = best.capabilities().filesystem >= nemo_sandbox::Guarantee::Weak;
    out!("sandbox for tool commands (`nemo run --sandbox enforce|best-effort|off`):");
    out!(
        "  default mode: enforce; `nemo run` {}",
        if confines {
            "will confine commands"
        } else {
            "will REFUSE to start (exit 2): no backend confines the filesystem; use --sandbox best-effort|off to run unconfined"
        }
    );
    out!("  selected {}", guarantees(&*nemo_sandbox::detect()));
    out!(
        "  network (`nemo run --network deny|loopback|allow`, default deny): {}",
        network_support(&*best)
    );
    #[cfg(target_os = "macos")]
    if let Some(b) = nemo_sandbox::SeatbeltBackend::detect() {
        out!(
            "  seatbelt available: true; network {}",
            network_support(&b)
        );
    } else {
        out!("  seatbelt available: false");
    }
    #[cfg(target_os = "linux")]
    {
        let l = nemo_sandbox::LinuxLandlockBackend::detect();
        let w = nemo_sandbox::BwrapBackend::detect();
        for (n, b) in [
            ("landlock", l.as_ref().map(|b| network_support(b))),
            ("bwrap", w.as_ref().map(|b| network_support(b))),
        ] {
            match b {
                Some(s) => out!("  {n} available: true; network {s}"),
                None => out!("  {n} available: false"),
            }
        }
    }
    out!(
        "  loopback: macOS exposes HOST loopback services; bwrap = private loopback; landlock/windows unsupported"
    );

    match nemo_browser::discover(env, nemo_browser::Os::current(), &|p| p.exists()) {
        Ok(p) => out!("browser: {}", p.display()),
        Err(e) => out!("browser: not found ({e})"),
    }
    out!(
        "git: {}",
        git_version(env).unwrap_or_else(|| "not found on PATH".into())
    );
    for v in [
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "ALL_PROXY",
        "NO_PROXY",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
    ] {
        out!(
            "{v}: {}",
            if env.get_nonempty(v).is_some() {
                "set"
            } else {
                "unset"
            }
        );
    }

    let h = homes(env)?;
    out!(
        "NEMO_HOME: {} (exists: {})",
        h.nemo.display(),
        h.nemo.is_dir()
    );
    match load_config(env, &h, cwd, ProjectTrust::Untrusted) {
        Ok(cfg) => {
            report_config(&cfg);
            report_discovery(env, &h, &cfg, cwd);
        }
        Err(e) => {
            out!("config: INVALID: {e}");
            code = 2;
        }
    }
    Ok(code)
}

fn report_config(cfg: &Config) {
    out!(
        "config: ok, {} provider(s), default model: {}",
        cfg.providers.len(),
        cfg.model.as_deref().unwrap_or("none")
    );
    for w in &cfg.warnings {
        out!("  warning: {}", san(w));
    }
}

/// Settings, MCP, skills and agents as the next `nemo run` would see them for `cwd` with an
/// untrusted project. Counts and names only, never a value.
fn report_discovery(env: &Env, h: &crate::Homes, cfg: &Config, cwd: Option<&Path>) {
    use nemo_settings::{LoadContext, load_settings};
    let none = h.nemo.join("no-project");
    let cwd = cwd.unwrap_or(&none);
    let st = load_settings(
        &LoadContext {
            cwd,
            nemo_home: &h.nemo,
            user_home: &h.user,
            env,
        },
        ProjectTrust::Untrusted,
    );
    let file = |p: std::path::PathBuf| {
        format!(
            "{} ({})",
            p.display(),
            if p.is_file() { "present" } else { "absent" }
        )
    };
    out!("settings.json:");
    out!("  user file: {}", file(h.nemo.join("settings.json")));
    out!(
        "  project file: {}",
        file(st.project_root.join(".nemo").join("settings.json"))
    );
    let p = &st.permissions;
    out!(
        "  permissions: {} allow, {} deny, {} ask rule(s), default mode {}",
        p.allow.len(),
        p.deny.len(),
        p.ask.len(),
        p.default_mode
    );
    out!(
        "  hooks: {} event(s){}; env: {} variable(s) applied, {} refused (secret-shaped){}",
        st.hooks.len(),
        if st.disable_all_hooks {
            " (all disabled)"
        } else {
            ""
        },
        st.env.len(),
        st.env_refused.len(),
        if st.env_refused.is_empty() {
            String::new()
        } else {
            format!(": {}", san(&st.env_refused.join(", ")))
        }
    );
    let v = crate::verify_cfg::parse(&st.nemo.verification);
    out!(
        "  nemo section: verification {} key(s), {} required check(s)",
        st.nemo.verification.len(),
        v.checks.len()
    );
    for d in st.diagnostics.iter().chain(&v.diagnostics) {
        out!("  warning: {}", san(d));
    }

    let plan = crate::mcp::plan(cfg, &st, &st, cwd, ProjectTrust::Untrusted, &[]);
    out!(
        "mcp: {} server(s) would start, {} skipped (never started by doctor)",
        plan.servers.len(),
        plan.skipped.len()
    );
    for s in &plan.servers {
        out!(
            "  {} [{}] {}",
            san(&s.name),
            s.scope.as_deref().unwrap_or("?"),
            san(&crate::mcp::describe(s)),
        );
    }
    for (n, why) in &plan.skipped {
        out!("  {}: skipped, {}", san(n), san(why));
    }
    let pass = crate::mcp::read_passthrough(&h.nemo.join("settings.json"));
    out!(
        "  env passthrough for ${{VAR}} (user scope only): {}",
        if pass.is_empty() {
            "none".into()
        } else {
            san(&pass.join(", "))
        }
    );

    let root = &st.project_root;
    let skills = nemo_tools::discover_skills(env, root);
    let proj = skills.iter().filter(|s| !s.trusted).count();
    out!(
        "skills: {} ({} untrusted project skill(s))",
        skills.len(),
        proj
    );
    for s in &skills {
        out!(
            "  {}{}",
            san(&s.name),
            if s.trusted {
                ""
            } else {
                " [untrusted project]"
            }
        );
    }
    let ag = crate::agents::load(&h.nemo, root, ProjectTrust::Untrusted);
    out!(
        "agents: {} user, {} project definition(s) ({} project not loaded: project untrusted)",
        ag.user,
        ag.project,
        ag.skipped.len()
    );
}
