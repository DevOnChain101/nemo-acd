//! `nemo run`: one headless task. Exit codes follow docs/specs/events.md §6.

use std::collections::BTreeMap;
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use clap::{Args, ValueEnum};
use nemo_config::{Config, Env, LoadContext, ProjectTrust, parse_model, resolve_credential};
use nemo_core::{AgentSession, BudgetUsage, SessionStatus, WorkspaceContext};
use nemo_events::{ArtifactRef, Event, SessionEnd, TaskStatus};
use nemo_harness::{
    Approval, ApprovalRequest, Approver, BoxFut, EventSink, Harness, HarnessConfig, Pricing, Scope,
    TaskOutcome, TaskRequest, ToolBox, budget_from_settings, from_mode, sanitize_for_display,
};
use nemo_hooks::Hooks;
use nemo_mcp::McpManager;
use nemo_permissions::{Mode, PermissionRules, Permissions};
use nemo_providers::{ModelProvider, NetSettings, ProviderSpec, RetryPolicy};
use nemo_sandbox::{NetworkPolicy, SandboxBackend, SandboxPolicy, SandboxedCommand};
use nemo_session::{NewSession, Retention, Store as SessionStore, StoreOptions};
use nemo_settings::load_settings;
use nemo_terminal::{Guarantee, SandboxMode, TerminalRuntime};
use nemo_tools::{Registry, standard_policy_git};
use nemo_verification::{CompletionGate, Criterion, VerifyRunner};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::ctx::{NemoContext, PutArtifact};
use crate::events::{Persist, Store};
use crate::taskdb::TaskDb;
use crate::transcript::Transcript;
use crate::wt::Worktree;
use crate::{homes, load_config};

#[derive(Clone, Copy, ValueEnum)]
pub enum ModeArg {
    Default,
    #[value(name = "acceptEdits")]
    AcceptEdits,
    Plan,
    #[value(name = "dontAsk")]
    DontAsk,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum SandboxArg {
    /// Refuse to run commands that cannot be confined (default).
    Enforce,
    /// Confine when the OS backend can, otherwise run unconfined.
    BestEffort,
    /// No confinement at all.
    Off,
}

impl SandboxArg {
    pub fn mode(self) -> SandboxMode {
        match self {
            Self::Enforce => SandboxMode::Enforce,
            Self::BestEffort => SandboxMode::BestEffort,
            Self::Off => SandboxMode::Off,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
pub enum NetworkArg {
    /// No network (default).
    Deny,
    /// Loopback only (dev server + tests in one sandbox). macOS: reaches the host's loopback
    /// services; Linux bwrap: private loopback; Landlock: unsupported.
    Loopback,
    /// Unrestricted network (loud warning).
    Allow,
}

impl NetworkArg {
    pub fn policy(self) -> NetworkPolicy {
        match self {
            Self::Deny => NetworkPolicy::Deny,
            Self::Loopback => NetworkPolicy::Loopback,
            Self::Allow => NetworkPolicy::Allow,
        }
    }
}

/// `Enforce` needs a backend that confines the filesystem and accepts the policy; `Err` is
/// the message for the exit-2 refusal.
pub(crate) fn sandbox_preflight(
    backend: &dyn SandboxBackend,
    policy: Option<&nemo_terminal::SandboxPolicy>,
) -> Result<(), String> {
    let hint = "run `nemo doctor` to see the sandbox backends, or pass --sandbox best-effort|off to run unconfined";
    if backend.capabilities().filesystem < Guarantee::Weak {
        return Err(format!(
            "no OS sandbox is available on this machine ({}); {hint}",
            backend.name()
        ));
    }
    let policy = policy.ok_or_else(|| {
        format!("cannot build the sandbox policy (artifact dir not writable); {hint}")
    })?;
    if backend.network_guarantee(&policy.network) < Guarantee::Weak {
        return Err(format!(
            "the {} sandbox cannot honour --network {}; {hint}",
            backend.name(),
            match policy.network {
                NetworkPolicy::Loopback => "loopback",
                _ => "deny",
            }
        ));
    }
    backend
        .wrap(policy, &SandboxedCommand::default())
        .map(|_| ())
        .map_err(|e| {
            format!(
                "the {} sandbox rejects the standard policy: {e}; {hint}",
                backend.name()
            )
        })
}

/// Flags shared by `nemo run` and the interactive app.
#[derive(Args, Clone)]
pub struct Common {
    /// Command confinement: enforce (default), best-effort, or off (loud warning).
    #[arg(long, value_enum, default_value = "enforce")]
    pub sandbox: SandboxArg,
    /// Network for tool commands: deny (default), loopback, or allow (loud warning).
    #[arg(long, value_enum, default_value = "deny")]
    pub network: NetworkArg,
    /// Extra read-only path for commands, e.g. a toolchain dir. Must exist; $HOME, filesystem
    /// roots and credential-store directories (or their ancestors) are refused (repeatable).
    #[arg(long, value_name = "PATH")]
    pub sandbox_read: Vec<PathBuf>,
    /// Provider id (with --model NAME), or `fake:<script.json>` in test-provider builds.
    #[arg(long)]
    pub provider: Option<String>,
    /// `provider/model`, an alias, or a bare model name.
    #[arg(long)]
    pub model: Option<String>,
    #[arg(long, value_enum)]
    pub permission_mode: Option<ModeArg>,
    /// Trust `<cwd>/.nemo/config.json`, `.nemo/settings.json`, agents and (with approval) MCP servers.
    #[arg(long)]
    pub trust_project: bool,
    /// Start this project-scope MCP server (needs --trust-project; repeatable).
    #[arg(long, value_name = "NAME")]
    pub approve_mcp: Vec<String>,
    #[arg(long)]
    pub no_session_store: bool,
    /// Allow a tool call without asking, e.g. `--allow 'Bash(cargo test:*)'` (settings rule
    /// syntax; repeatable). Typed by you, so trusted; it cannot lift critical-path or
    /// protected-path protections.
    #[arg(long, value_name = "RULE")]
    pub allow: Vec<String>,
    /// Deny a tool call, e.g. `--deny 'Bash(git push:*)'` (settings rule syntax; repeatable).
    /// Deny wins over every allow.
    #[arg(long, value_name = "RULE")]
    pub deny: Vec<String>,
    /// Work in an isolated git worktree on branch `nemo/<session>`; left in place for review.
    #[arg(long)]
    pub worktree: bool,
    /// With --worktree: remove the worktree at the end when nothing changed.
    #[arg(long, requires = "worktree")]
    pub cleanup_worktree: bool,
}

#[derive(Args)]
pub struct RunArgs {
    /// Workspace directory (required; no implicit cwd).
    #[arg(long)]
    cwd: PathBuf,
    #[arg(long, conflicts_with = "prompt_file")]
    prompt: Option<String>,
    #[arg(long)]
    prompt_file: Option<PathBuf>,
    #[arg(long)]
    max_turns: Option<u32>,
    #[arg(long)]
    max_tool_calls: Option<u32>,
    /// Needs a price for the model: `nemo.pricing` in the user settings.json (config.md §6).
    #[arg(long)]
    max_cost_usd: Option<f64>,
    #[arg(long)]
    max_wall_secs: Option<u64>,
    /// JSON array of `{id?, text, check?}` (or plain strings).
    #[arg(long, alias = "criteria")]
    criteria_file: Option<PathBuf>,
    /// Also write the event stream to this file.
    #[arg(long)]
    events_out: Option<PathBuf>,
    #[command(flatten)]
    common: Common,
}

#[derive(Default, Clone)]
pub struct Limits {
    pub max_turns: Option<u32>,
    pub max_tool_calls: Option<u32>,
    pub max_cost_usd: Option<f64>,
    pub max_wall_secs: Option<u64>,
}

pub enum Front {
    /// Events go to stdout (and `tee`); asks are denied with an audit trail.
    Headless {
        prompt: String,
        tee: Option<std::fs::File>,
        /// Set (and the run cancelled) when stdout is closed by the reader.
        closed: Arc<std::sync::atomic::AtomicBool>,
    },
    /// Events go to the UI; the UI approves.
    Tui,
}

/// Everything a front end needs, built once by [`prepare`].
pub struct Prepared {
    pub harness: Arc<Harness>,
    pub session: AgentSession,
    pub sid: String,
    pub cancel: CancellationToken,
    pub sink: EventSink,
    pub rt: Arc<TerminalRuntime>,
    pub store: Option<Store>,
    pub transcript: Option<Arc<Transcript>>,
    pub tokio: tokio::runtime::Runtime,
    pub mcp: Option<Arc<McpManager>>,
    pub notices: Vec<String>,
    pub worktree: Option<Worktree>,
    /// Required checks from `nemo.verification.checks`.
    pub criteria: Vec<Criterion>,
    pub artifact_root: PathBuf,
    pub label: String,
    pub models: Vec<String>,
    pub sandbox: nemo_tui::SandboxStatus,
    pub link: Option<nemo_tui::UiLink>,
    pub homes: crate::Homes,
}

/// Rows of `{"nemo":{"pricing":{"<provider/model>"|"<model>": {input_per_mtok, output_per_mtok}}}}`
/// from the USER settings file only (a repository must not be able to price a model at zero).
fn pricing(user_settings: &Path, label: &str, model: &str) -> Option<Pricing> {
    let raw = std::fs::read(user_settings)
        .ok()
        .filter(|b| b.len() <= 1 << 20)?;
    let v: Value = serde_json::from_slice(&raw).ok()?;
    parse_pricing(&v, label, model)
}

fn parse_pricing(v: &Value, label: &str, model: &str) -> Option<Pricing> {
    let table = v.get("nemo")?.get("pricing")?;
    let e = table.get(label).or_else(|| table.get(model))?;
    let num = |k: &str| {
        e.get(k)
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite() && *n >= 0.0)
    };
    Some(Pricing {
        input_per_mtok: num("input_per_mtok")?,
        output_per_mtok: num("output_per_mtok")?,
    })
}

/// `--allow` / `--deny` values: non-empty, one line, `Tool` or `Tool(content)`.
fn rules_arg(v: &[String], flag: &str) -> Result<Vec<String>, String> {
    v.iter()
        .map(|r| {
            let r = r.trim();
            let name = r.split('(').next().unwrap_or("");
            let well_formed = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
                && (r.len() == name.len() || r.ends_with(')'))
                && r.len() <= 2000
                && !r.chars().any(char::is_control);
            if well_formed {
                Ok(r.to_string())
            } else {
                Err(format!(
                    "{flag} {}: expected a rule like Bash(cargo test:*), Edit or mcp__server__tool",
                    sanitize_for_display(r)
                ))
            }
        })
        .collect()
}

/// A name/value that `Command::env` accepts.
fn env_pair_ok(k: &str, v: &str) -> bool {
    !k.is_empty() && !k.contains(['=', '\0']) && !v.contains('\0')
}

/// Headless: nobody can answer, so every ask is refused, and the harness still records the
/// request and the resolution (`reviewer: policy:non-interactive`) in the event stream.
struct AuditDeny;

impl Approver for AuditDeny {
    fn approve<'a>(&'a self, _: ApprovalRequest, _: &'a CancellationToken) -> BoxFut<'a, Approval> {
        Box::pin(async {
            Approval {
                reviewer: "policy:non-interactive",
                ..Approval::auto_denied()
            }
        })
    }
}

/// Warnings for the sandbox/network flags; shown at start and sent as `notice` events.
pub fn sandbox_notices(mode: SandboxMode, network: NetworkArg) -> Vec<String> {
    let mut n = vec![];
    match mode {
        SandboxMode::Off => n.push(
            "unsandboxed_by_request: --sandbox off, commands run with the agent's full privileges".to_string(),
        ),
        SandboxMode::BestEffort => n.push(
            "sandbox best-effort: commands run unconfined (unsandboxed_by_request) when the OS backend cannot confine".to_string(),
        ),
        SandboxMode::Enforce => {}
    }
    if matches!(network, NetworkArg::Allow) {
        n.push("--network allow: commands have unrestricted network access".into());
    }
    n
}

pub fn sandbox_status(mode: SandboxMode, unsandboxed: bool) -> nemo_tui::SandboxStatus {
    let b = nemo_sandbox::detect();
    let c = b.capabilities();
    let g = |x: Guarantee| format!("{x:?}").to_lowercase();
    nemo_tui::SandboxStatus {
        mode: match mode {
            SandboxMode::Enforce => "enforce",
            SandboxMode::BestEffort => "best-effort",
            SandboxMode::Off => "off",
        }
        .into(),
        backend: b.name().into(),
        filesystem: g(c.filesystem),
        network: g(c.network),
        unsandboxed_by_request: unsandboxed,
    }
}

#[allow(clippy::too_many_lines)]
pub fn prepare(
    c: &Common,
    cwd: &Path,
    env: &Env,
    limits: &Limits,
    front: Front,
    resume: Option<&nemo_tui::Resume>,
) -> Result<Prepared, String> {
    let (cli_allow, cli_deny) = (
        rules_arg(&c.allow, "--allow")?,
        rules_arg(&c.deny, "--deny")?,
    );
    let h = homes(env)?;
    let nemo_deny = [h.nemo.join("auth.json"), h.nemo.join("settings.json")];
    // Before anything is created (worktree, store): a bad grant is a usage error.
    let reads = {
        let mut probe = SandboxPolicy::standard(cwd, Path::new("/nemo-probe-tmp"));
        probe.deny_paths.extend(nemo_deny.clone());
        c.sandbox_read
            .iter()
            .map(|r| {
                probe
                    .check_read_root(r)
                    .map_err(|e| format!("--sandbox-read {e}"))
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    let trust = if c.trust_project {
        ProjectTrust::Trusted
    } else {
        ProjectTrust::Untrusted
    };
    let lc = LoadContext {
        cwd,
        nemo_home: &h.nemo,
        user_home: &h.user,
        env,
    };
    let cfg = load_config(env, &h, Some(cwd), trust)?;
    let user_cfg = if c.trust_project {
        load_config(env, &h, Some(cwd), ProjectTrust::Untrusted)?
    } else {
        cfg.clone()
    };
    let settings = load_settings(&lc, trust);
    let user_settings = if c.trust_project {
        load_settings(&lc, ProjectTrust::Untrusted)
    } else {
        settings.clone()
    };
    let vs = crate::verify_cfg::parse(&settings.nemo.verification);
    for w in cfg
        .warnings
        .iter()
        .chain(&settings.diagnostics)
        .chain(&vs.diagnostics)
    {
        eprintln!("nemo: warning: {}", sanitize_for_display(w));
    }
    if !settings.env_refused.is_empty() {
        eprintln!(
            "nemo: warning: settings env: refused secret-shaped name(s): {}",
            sanitize_for_display(&settings.env_refused.join(", "))
        );
    }
    let res = resolve_provider(c, &cfg, &h.nemo, env)?;

    let price = pricing(&h.nemo.join("settings.json"), &res.label, &res.model);
    if limits.max_cost_usd.is_some() && price.is_none() {
        return Err(format!(
            "--max-cost-usd needs a price for {}: add nemo.pricing[\"{}\"] = {{\"input_per_mtok\": N, \"output_per_mtok\": N}} to {} (see docs/specs/config.md §6)",
            res.label,
            res.label,
            h.nemo.join("settings.json").display()
        ));
    }

    let mode = match c.permission_mode {
        Some(ModeArg::Default) => Mode::Default,
        Some(ModeArg::AcceptEdits) => Mode::AcceptEdits,
        Some(ModeArg::Plan) => Mode::Plan,
        Some(ModeArg::DontAsk) => Mode::DontAsk,
        None => match Mode::parse(&settings.permissions.default_mode) {
            Some(m @ (Mode::Default | Mode::AcceptEdits | Mode::Plan | Mode::DontAsk)) => m,
            _ => Mode::Default, // auto/bypassPermissions are not available headless
        },
    };
    let mut budget = resume.map_or_else(
        || budget_from_settings(&settings),
        |r| r.record.budget.clone(),
    );
    if let Some(v) = limits.max_turns {
        budget.max_turns = v;
    }
    if let Some(v) = limits.max_tool_calls {
        budget.max_tool_calls = v;
    }
    if let Some(v) = vs.max_attempts {
        budget.max_verify_attempts = v;
    }
    budget.max_cost_usd = limits.max_cost_usd;
    budget.max_wall_ms = limits.max_wall_secs.map(|s| s.saturating_mul(1000));

    let mut extra_dirs: Vec<PathBuf> = settings
        .permissions
        .additional_directories
        .iter()
        .filter_map(|d| match std::fs::canonicalize(d) {
            Ok(p) if Path::new(d).is_absolute() && p.is_dir() => Some(p),
            _ => {
                eprintln!("nemo: warning: ignored additional directory (not an existing absolute directory)");
                None
            }
        })
        .collect();

    let sid = resume.map_or_else(|| uuid::Uuid::new_v4().to_string(), |r| r.record.id.clone());
    let artifact_root = h.nemo.join("sessions").join(&sid);
    let (ws, worktree) = if let Some(r) = resume {
        (
            r.record
                .open_workspace(&h.nemo)
                .map_err(|e| e.to_string())?,
            None,
        )
    } else if c.worktree {
        if !extra_dirs.is_empty() {
            eprintln!("nemo: warning: additionalDirectories are ignored with --worktree");
            extra_dirs.clear();
        }
        let (w, ctx) = crate::wt::create(env, &h.nemo, cwd, &sid, artifact_root.clone())?;
        (ctx, Some(w))
    } else {
        (
            WorkspaceContext::open(
                format!("ws-{}", &sid[..8]),
                cwd,
                cwd,
                &extra_dirs,
                artifact_root.clone(),
                sid.clone(),
            )
            .map_err(|e| e.to_string())?,
            None,
        )
    };
    let session = match resume {
        Some(r) => AgentSession {
            id: sid.clone(),
            task_id: None,
            status: r.record.status,
            workspace: ws.clone(),
            model_config_id: res.label.clone(),
            permission_policy_id: r.record.permission_policy_id.clone(),
            permission_mode: r.record.permission_mode,
            event_cursor: r.record.event_cursor,
            budget: budget.clone(),
            usage: r.record.usage.clone(),
            created_at: r.record.created_at.clone(),
            updated_at: r.record.updated_at.clone(),
            parent: None,
        },
        None => AgentSession {
            id: sid.clone(),
            task_id: None,
            status: SessionStatus::Created,
            workspace: ws.clone(),
            model_config_id: res.label.clone(),
            permission_policy_id: "default".into(),
            permission_mode: from_mode(mode),
            event_cursor: 0,
            budget: budget.clone(),
            usage: BudgetUsage::default(),
            created_at: String::new(),
            updated_at: String::new(),
            parent: None,
        },
    };
    let ws = Arc::new(ws);

    let sandbox_mode = c.sandbox.mode();
    let net = c.network.policy();
    let git_dir = worktree.as_ref().map(|w| w.info.git_dir.clone());
    let build = Arc::new(move |ws: &nemo_core::WorkspaceContext| {
        let mut p = standard_policy_git(ws, net.clone(), git_dir.as_deref())?;
        p.deny_paths.extend(nemo_deny.clone());
        p.readable_roots.extend(reads.iter().cloned());
        Ok::<_, String>(p)
    });
    // Per call an Err becomes None, which the terminal runtime REFUSES under Enforce (it only
    // runs unconfined in best-effort/off); preflight below reports the Err once.
    let make_policy = {
        let build = build.clone();
        Arc::new(move |ws: &nemo_core::WorkspaceContext| build(ws).ok())
    };
    let policy = build(&ws);
    match (&policy, sandbox_mode) {
        (Err(e), SandboxMode::Enforce) => {
            return Err(format!("cannot build the sandbox policy: {e}"));
        }
        (Err(e), SandboxMode::BestEffort) => eprintln!(
            "nemo: warning: cannot build the sandbox policy ({e}); commands run unconfined"
        ),
        _ => {}
    }
    if sandbox_mode == SandboxMode::Enforce {
        sandbox_preflight(&*nemo_sandbox::detect(), policy.as_ref().ok())?;
    }
    let mut notices = sandbox_notices(sandbox_mode, c.network);
    for n in &notices {
        eprintln!("nemo: WARNING: {n}");
    }

    let store: Option<Store> = if c.no_session_store {
        None
    } else {
        let opts = StoreOptions {
            write_node_transcripts: settings.nemo.compat.write_node_transcripts.unwrap_or(true),
            secrets: res.secrets.clone(),
            ..StoreOptions::default()
        };
        let w = SessionStore::open(&h.nemo, opts)
            .and_then(|s| match resume {
                Some(_) => s.resume_session(&sid, true),
                None => s.create_session(NewSession {
                    id: Some(sid.clone()),
                    workspace: &ws,
                    model_config_id: res.label.clone(),
                    permission_mode: session.permission_mode,
                    permission_policy_id: "default".into(),
                    budget: budget.clone(),
                    parent: None,
                    title: None,
                }),
            })
            .map_err(|e| {
                format!("session store: {e} (use --no-session-store to run without it)")
            })?;
        Some(Arc::new(Mutex::new(w)))
    };
    let transcript = store.clone().map(|s| {
        let t = Transcript::new(s);
        t.skip(resume.map_or(0, |r| r.history.len()));
        Arc::new(t)
    });
    let derived = match (&store, &transcript) {
        (Some(_), Some(t)) => match TaskDb::open(&h.nemo, &sid, res.secrets.clone()) {
            Ok(db) => Some((db, t.clone())),
            Err(e) => {
                eprintln!("nemo: warning: task rows disabled: {e}");
                None
            }
        },
        _ => None,
    };
    let cancel = CancellationToken::new();
    let (out, tee, prompt, link, approver, closed): (
        Box<dyn std::io::Write + Send>,
        _,
        String,
        _,
        Arc<dyn Approver>,
        _,
    ) = match front {
        Front::Headless {
            prompt,
            tee,
            closed,
        } => (
            Box::new(std::io::stdout()),
            tee,
            prompt,
            None,
            Arc::new(AuditDeny),
            Some((closed, cancel.clone())),
        ),
        Front::Tui => {
            let l = nemo_tui::UiLink::new();
            let ap = l.approver();
            (
                Box::new(l.event_writer()),
                None,
                String::new(),
                Some(l),
                ap,
                None,
            )
        }
    };
    let sink = EventSink::new(
        Persist::new(
            out,
            tee,
            store.clone(),
            derived,
            resume.map_or(0, |r| r.record.event_cursor),
            closed,
        ),
        sid.clone(),
        res.secrets.clone(),
    );

    // settings.env: opt-in additions on top of the allowlisted child env (config.md §6). A
    // variable the process env already supplies to children wins, as in Node's agentEnv.
    let mut child_env: BTreeMap<String, String> = env
        .child_env()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    for (k, v) in &settings.env {
        if env_pair_ok(k, v) && !child_env.contains_key(k) {
            child_env.insert(k.clone(), v.clone());
        }
    }
    let rt = Arc::new(TerminalRuntime::new(sandbox_mode));
    rt.open_session_sandboxed(&sid, ws.clone(), child_env.clone(), policy.ok());

    let rules = PermissionRules {
        allow: settings
            .permissions
            .allow
            .iter()
            .chain(&cli_allow)
            .cloned()
            .collect(),
        deny: settings
            .permissions
            .deny
            .iter()
            .chain(&cli_deny)
            .cloned()
            .collect(),
        ask: settings.permissions.ask.clone(),
        additional_directories: extra_dirs,
    };
    // `interactive` lets asks reach the approver, so each one leaves approval events.
    let mut perms = Permissions::new(
        rules,
        nemo_permissions::Env::new(cwd, &h.user, mode).interactive(true),
    );
    perms.block_outside_reads = settings.permissions.block_reads_outside_working_directories;

    let mut hcfg = HarnessConfig::new(res.model.clone());
    hcfg.max_output_tokens = cfg.max_tokens as u32;
    hcfg.env = child_env;
    hcfg.pricing = price;
    let (auto_compact, diag) = crate::ctx::read_auto_compact(&h.nemo.join("settings.json"));
    for d in &diag {
        eprintln!("nemo: warning: {d}");
    }
    hcfg.auto_compact = auto_compact;
    let (shell, diag) = crate::ctx::read_shell_settings(&h.nemo.join("settings.json"));
    for d in &diag {
        eprintln!("nemo: warning: {d}");
    }
    hcfg.shell = shell;

    let put: PutArtifact = match &store {
        Some(s) => {
            let s = s.clone();
            Box::new(move |b| {
                s.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .put_artifact("summary", "text/plain", b, Retention::Session)
                    .map_err(|e| e.to_string())
            })
        }
        None => artifact_to_dir(artifact_root.clone()),
    };
    let user_files = [h.nemo.join("NEMO.md")];
    let instructions = nemo_context::load_instructions(
        &ws,
        &user_files,
        Some(&h.user),
        &nemo_context::DEFAULT_NAMES,
        &nemo_context::InstructionLimits::default(),
    );
    let skills = crate::ctx::visible_skills(
        nemo_tools::discover_skills(env, &ws.root),
        trust == ProjectTrust::Trusted,
    );
    let mut context = NemoContext::new(
        instructions,
        &prompt,
        res.provider.clone(),
        res.model.clone(),
        cancel.clone(),
        put,
    )
    .with_skills(&skills);
    if let Some(t) = &transcript {
        context = context.with_transcript(t.clone());
    }
    let agents = crate::agents::load(&h.nemo, &settings.project_root, trust);
    if !agents.skipped.is_empty() {
        let w = format!(
            "{} project agent definition(s) not loaded (project not trusted)",
            agents.skipped.len()
        );
        eprintln!("nemo: warning: {w}");
        notices.push(w);
    }

    // Everything fallible is done: start MCP servers last so an early error never leaves one.
    let tokio_rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let mut mplan = crate::mcp::plan(
        &user_cfg,
        &settings,
        &user_settings,
        cwd,
        trust,
        &c.approve_mcp,
    );
    mplan.env_passthrough = crate::mcp::read_passthrough(&h.nemo.join("settings.json"));
    mplan.guard = crate::mcp::read_guard(
        &h.nemo.join("settings.json"),
        (trust == ProjectTrust::Trusted)
            .then(|| settings.project_root.join(".nemo").join("settings.json"))
            .as_deref(),
    );
    mplan.artifact_dir = Some(artifact_root.join("mcp"));
    // Printed now (before any TUI alt screen) and kept in `notices`, which become `notice`
    // events (headless) or TUI startup notes, so approvals stay visible after the screen clears.
    for n in &mplan.notices {
        let n = sanitize_for_display(n);
        eprintln!("nemo: {n}");
        notices.push(n);
    }
    for (n, why) in &mplan.skipped {
        let w = format!(
            "MCP server \"{}\" not started: {}",
            sanitize_for_display(n),
            sanitize_for_display(why)
        );
        eprintln!("nemo: warning: {w}");
        notices.push(w);
    }
    let (mcp, mcp_attach) = if mplan.servers.is_empty() {
        (None, None)
    } else {
        let (m, t, warns) = tokio_rt.block_on(crate::mcp::start(&mplan, env, &ws.root));
        for w in warns {
            eprintln!("nemo: warning: {}", sanitize_for_display(&w));
        }
        (Some(m), Some(t))
    };

    let mut toolbox =
        ToolBox::new(Registry::with_terminal_policy(rt.clone(), make_policy).with_new_tools());
    if let Some(a) = mcp_attach {
        toolbox = a.apply(toolbox);
    }
    let gate_cfg = vs.gate(budget.max_verify_attempts);
    let rt_gate = rt.clone();
    let mut harness = Harness::new(res.provider.clone(), toolbox, perms, sink.clone(), hcfg)
        .with_approver(approver)
        .with_agents(agents.table)
        .with_context(Arc::new(context))
        .with_gate(Box::new(move |s| {
            CompletionGate::new(
                VerifyRunner::new(rt_gate.clone(), &s.id, Arc::new(s.workspace.clone())),
                gate_cfg.clone(),
            )
        }));
    if let Some(l) = &link {
        harness = harness.with_input(l.input());
    }
    if !settings.disable_all_hooks && !settings.hooks.is_empty() {
        harness = harness.with_hooks(Arc::new(Hooks::new(
            rt.clone(),
            sid.clone(),
            ws.clone(),
            settings.hooks.clone(),
        )));
    }

    eprintln!(
        "nemo: session {sid} model {} cwd {} mode {}",
        res.label,
        ws.cwd.display(),
        mode.as_str()
    );
    if link.is_some() {
        // Event log / replay copy; the UI also gets `notices` as startup notes (tui.rs) and shows
        // each text once, so it no longer depends on this channel buffering.
        for n in &notices {
            sink.emit(
                &Scope::default(),
                Event::Notice {
                    level: "warning".into(),
                    text: n.clone(),
                },
            );
        }
    }
    let models = nemo_config::list_models(&cfg, None)
        .into_iter()
        .map(|m| m.value)
        .collect();
    Ok(Prepared {
        harness: Arc::new(harness),
        session,
        sid,
        cancel,
        sink,
        rt,
        store,
        transcript,
        tokio: tokio_rt,
        mcp,
        notices,
        worktree,
        criteria: vs.checks,
        artifact_root,
        label: res.label,
        models,
        sandbox: sandbox_status(sandbox_mode, sandbox_mode != SandboxMode::Enforce),
        link,
        homes: h,
    })
}

pub fn run(a: RunArgs, env: Env) -> Result<u8, String> {
    let prompt = read_prompt(&a)?;
    let mut criteria = a
        .criteria_file
        .as_deref()
        .map(read_criteria)
        .transpose()?
        .unwrap_or_default();
    if a.max_cost_usd.is_some_and(|c| !c.is_finite() || c <= 0.0) {
        return Err("--max-cost-usd must be a positive number".into());
    }
    let cwd =
        nemo_core::canonicalize(&a.cwd).map_err(|e| format!("--cwd {}: {e}", a.cwd.display()))?;
    if !cwd.is_dir() {
        return Err(format!("--cwd {} is not a directory", cwd.display()));
    }
    let tee = a
        .events_out
        .as_ref()
        .map(|p| create_private(p).map_err(|e| format!("--events-out {}: {e}", p.display())))
        .transpose()?;
    let limits = Limits {
        max_turns: a.max_turns,
        max_tool_calls: a.max_tool_calls,
        max_cost_usd: a.max_cost_usd,
        max_wall_secs: a.max_wall_secs,
    };
    let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let p = prepare(
        &a.common,
        &cwd,
        &env,
        &limits,
        Front::Headless {
            prompt: prompt.clone(),
            tee,
            closed: closed.clone(),
        },
        None,
    )?;
    criteria.extend(p.criteria.iter().cloned());
    let Prepared {
        harness,
        session,
        sid,
        cancel,
        sink,
        rt,
        store,
        transcript,
        tokio: tokio_rt,
        mcp,
        notices,
        worktree,
        artifact_root,
        ..
    } = p;
    let cleanup = a.common.cleanup_worktree;
    let code = tokio_rt.block_on(async {
        on_signals(cancel.clone()).map_err(|e| format!("signal handlers: {e}"))?;
        let (store2, sink2, h2) = (store.clone(), sink.clone(), harness.clone());
        let task = tokio::spawn(async move {
            let mut session = session;
            let r = async {
                h2.start_session(&mut session, "startup")?;
                for n in notices {
                    sink2.emit(
                        &Scope::default(),
                        Event::Notice {
                            level: "warning".into(),
                            text: n,
                        },
                    );
                }
                let out = h2
                    .run_task(
                        &mut session,
                        TaskRequest {
                            request: prompt,
                            criteria,
                            history: vec![],
                            display_text: None,
                        },
                        cancel,
                    )
                    .await?;
                if let Some(w) = &worktree {
                    sink2.emit(
                        &Scope {
                            task_id: Some(out.task_id.clone()),
                            ..Scope::default()
                        },
                        Event::Notice {
                            level: "info".into(),
                            text: w.finish(cleanup),
                        },
                    );
                }
                h2.end_session(&out);
                Ok::<_, nemo_harness::HarnessError>(out)
            }
            .await;
            if let (Some(s), Ok(out)) = (&store2, &r) {
                finish_store(s, transcript.as_deref(), out, &session.usage);
            }
            r
        });
        let code = match task.await {
            Ok(Ok(out)) => {
                eprintln!("nemo: task {:?}: {}", out.status, out.reason);
                if let Some(h) = blocked_hint(&out) {
                    eprintln!("{h}");
                }
                exit_code(out.status)
            }
            Ok(Err(e)) => {
                eprintln!("nemo: internal error: {e}");
                70
            }
            Err(e) => {
                eprintln!("nemo: internal error: {e}");
                sink.emit(
                    &Scope::default(),
                    Event::SessionFailed(SessionEnd {
                        reason: "internal_error".into(),
                        error: Some(e.to_string()),
                    }),
                );
                70
            }
        };
        if let Some(m) = &mcp {
            m.close().await; // stdio servers get EOF, then their process group is killed
        }
        Ok::<u8, String>(code)
    })?;
    rt.close_session(&sid); // kills leftover background processes
    drop(harness);
    let code = if closed.load(std::sync::atomic::Ordering::SeqCst) {
        EXIT_STDOUT_CLOSED
    } else {
        code
    };
    if a.common.no_session_store {
        let _ = std::fs::remove_dir_all(&artifact_root);
    }
    Ok(code)
}

/// Flushes the conversation tail and the usage to the store.
pub fn finish_store(
    store: &Store,
    tr: Option<&Transcript>,
    out: &TaskOutcome,
    usage: &BudgetUsage,
) {
    if let Some(t) = tr {
        t.sync(&out.messages);
    }
    if let Err(e) = store
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .update_usage(usage)
    {
        eprintln!("nemo: warning: session store: {e}");
    }
}

fn read_prompt(a: &RunArgs) -> Result<String, String> {
    let p = match (&a.prompt, &a.prompt_file) {
        (Some(p), _) => p.clone(),
        (_, Some(f)) => {
            std::fs::read_to_string(f).map_err(|e| format!("--prompt-file {}: {e}", f.display()))?
        }
        _ => {
            if std::io::stdin().is_terminal() {
                return Err("no prompt: pass --prompt, --prompt-file or pipe it on stdin".into());
            }
            let mut s = String::new();
            std::io::stdin()
                .read_to_string(&mut s)
                .map_err(|e| format!("stdin: {e}"))?;
            s
        }
    };
    if p.trim().is_empty() {
        return Err("empty prompt".into());
    }
    Ok(p)
}

fn read_criteria(f: &Path) -> Result<Vec<Criterion>, String> {
    let bad = |e: String| format!("--criteria-file {}: {e}", f.display());
    let raw = std::fs::read_to_string(f).map_err(|e| bad(e.to_string()))?;
    let items: Vec<Value> = serde_json::from_str(&raw).map_err(|e| bad(e.to_string()))?;
    items
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let text = v
                .as_str()
                .or_else(|| v.get("text").and_then(Value::as_str))
                .ok_or_else(|| bad(format!("item {i}: missing text")))?;
            let id = v
                .get("id")
                .and_then(Value::as_str)
                .map_or(format!("c{}", i + 1), String::from);
            Ok(Criterion::user(
                &id,
                text,
                v.get("check").and_then(Value::as_str),
            ))
        })
        .collect()
}

struct Resolved {
    provider: Arc<dyn ModelProvider>,
    model: String,
    label: String,
    secrets: Vec<String>,
}

fn resolve_provider(
    a: &Common,
    cfg: &Config,
    nemo_home: &Path,
    env: &Env,
) -> Result<Resolved, String> {
    if let Some(script) = a.provider.as_deref().and_then(|p| p.strip_prefix("fake:")) {
        #[cfg(feature = "test-provider")]
        {
            let model = a.model.clone().unwrap_or_else(|| "fake-model".into());
            return Ok(Resolved {
                provider: crate::fake::load(Path::new(script))?,
                label: format!("fake/{model}"),
                model,
                secrets: vec![],
            });
        }
        #[cfg(not(feature = "test-provider"))]
        return Err(format!(
            "--provider fake:{script} needs a build with the test-provider feature"
        ));
    }
    let spec = match (&a.provider, &a.model) {
        (Some(p), Some(m)) if !m.contains('/') => Some(format!("{p}/{m}")),
        (Some(p), None) => cfg
            .model
            .clone()
            .filter(|m| m.starts_with(&format!("{p}/")))
            .or_else(|| {
                cfg.providers
                    .get(p)?
                    .models
                    .as_ref()?
                    .first()
                    .map(|m| format!("{p}/{m}"))
            }),
        (_, m) => m.clone(),
    };
    let parsed = parse_model(spec.as_deref(), cfg).map_err(|_| {
        "no model configured: pass --model provider/model or set NEMO_MODEL".to_string()
    })?;
    let id = parsed.provider_id.ok_or("no provider configured")?;
    let p = cfg
        .providers
        .get(&id)
        .ok_or_else(|| format!("unknown provider \"{id}\""))?;
    let cred = resolve_credential(&id, Some(p), nemo_home, env);
    let secrets = cred.iter().map(|c| c.secret.clone()).collect();
    let provider = ProviderSpec::from_config(&id, p, cred.as_ref())
        .and_then(|s| s.connect(env, &NetSettings::default(), RetryPolicy::default()))
        .map_err(|e| e.message().to_string())?;
    Ok(Resolved {
        provider,
        label: format!("{id}/{}", parsed.model),
        model: parsed.model,
        secrets,
    })
}

/// stdout was closed by the reader; the task was cancelled and its events are in the store.
pub const EXIT_STDOUT_CLOSED: u8 = 141;

/// `nemo: blocked: approval needed for ... - rerun with --allow ... or in the TUI`.
fn blocked_hint(out: &TaskOutcome) -> Option<String> {
    if out.needed_permissions.is_empty() {
        return None;
    }
    // Blocked is the usual outcome; a model that worked around the denial still gets told.
    let lead = if out.status == TaskStatus::Blocked {
        "nemo: blocked: approval needed for"
    } else {
        "nemo: note: approval was needed but unavailable for"
    };
    let rules = &out.needed_permissions;
    // The rule text derives from the model's command: never offer it paste-ready unless it is inert
    // in a shell (no quote, expansion, separator, redirection or control character).
    let paste_safe = |r: &str| {
        !r.chars()
            .any(|c| c.is_control() || "'\"$`\\;|&<>".contains(c))
    };
    let flags: Vec<String> = rules
        .iter()
        .filter(|r| paste_safe(r))
        .map(|r| format!("--allow '{r}'"))
        .collect();
    let names = rules
        .iter()
        .map(|r| sanitize_for_display(r))
        .collect::<Vec<_>>()
        .join(", ");
    let how = if flags.len() == rules.len() {
        format!("rerun with {} or in the TUI", flags.join(" "))
    } else {
        "the rule text comes from the model's command, so review it and add it with --allow yourself, or approve in the TUI".to_string()
    };
    Some(format!("{lead} {names} \u{2014} {how}"))
}

fn exit_code(s: TaskStatus) -> u8 {
    match s {
        TaskStatus::Completed => 0,
        TaskStatus::Failed => 1,
        TaskStatus::Unverified => 3,
        TaskStatus::Blocked => 4,
        TaskStatus::Cancelled => 130,
        _ => 70,
    }
}

#[cfg(unix)]
fn on_signals(cancel: CancellationToken) -> std::io::Result<()> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut int = signal(SignalKind::interrupt())?;
    let mut term = signal(SignalKind::terminate())?;
    tokio::spawn(async move {
        tokio::select! { _ = int.recv() => {}, _ = term.recv() => {} }
        cancel.cancel();
        tokio::select! { _ = int.recv() => {}, _ = term.recv() => {} }
        std::process::exit(130); // second signal: stop waiting for a clean shutdown
    });
    Ok(())
}

#[cfg(not(unix))]
fn on_signals(cancel: CancellationToken) -> std::io::Result<()> {
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        cancel.cancel();
        let _ = tokio::signal::ctrl_c().await;
        std::process::exit(130);
    });
    Ok(())
}

/// Creates (or truncates) a file readable by the owner only; 0600 on unix even if it existed.
fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    let f = o.open(path)?;
    #[cfg(unix)]
    f.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    Ok(f)
}

fn artifact_to_dir(dir: PathBuf) -> PutArtifact {
    Box::new(move |bytes| {
        let id = uuid::Uuid::new_v4().to_string();
        std::fs::create_dir_all(dir.join("artifacts")).map_err(|e| e.to_string())?;
        let path = format!("artifacts/{id}.txt");
        create_private(&dir.join(&path))
            .and_then(|mut f| std::io::Write::write_all(&mut f, bytes))
            .map_err(|e| e.to_string())?;
        let sha256 = Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        Ok(ArtifactRef {
            artifact_id: id,
            path,
            size: bytes.len() as u64,
            sha256,
            mime: "text/plain".into(),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(rules: &[&str]) -> TaskOutcome {
        TaskOutcome {
            task_id: String::new(),
            status: TaskStatus::Blocked,
            reason: String::new(),
            verification_report_id: None,
            final_text: String::new(),
            usage: nemo_events::Usage {
                input_tokens: 0,
                output_tokens: 0,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
                cost_usd: None,
            },
            changed_files: vec![],
            needed_permissions: rules.iter().map(|r| r.to_string()).collect(),
            messages: vec![],
        }
    }

    #[test]
    fn blocked_hint_offers_only_shell_inert_rules() {
        let h = blocked_hint(&outcome(&["Bash(cargo test:*)"])).unwrap();
        assert!(h.contains("--allow 'Bash(cargo test:*)'"), "{h}");
        for bad in [
            "Bash(a'; rm -rf ~:*)",
            "Bash($(id):*)",
            "Bash(a`b`:*)",
            "Bash(a\\b)",
            "Bash(a|b)",
            "Bash(a\nb)",
        ] {
            let h = blocked_hint(&outcome(&[bad])).unwrap();
            assert!(!h.contains("--allow '"), "{h}");
            assert!(h.contains("comes from the model's command"), "{h}");
        }
    }

    #[test]
    fn preflight_refuses_loopback_the_backend_cannot_honour() {
        struct NoLoopback;
        impl SandboxBackend for NoLoopback {
            fn name(&self) -> &'static str {
                "fake"
            }
            fn capabilities(&self) -> nemo_terminal::SandboxCapabilities {
                nemo_terminal::SandboxCapabilities {
                    filesystem: Guarantee::Strong,
                    network: Guarantee::Strong,
                    process_tree: Guarantee::Strong,
                    notes: vec![],
                }
            }
            fn wrap(
                &self,
                _: &nemo_terminal::SandboxPolicy,
                c: &SandboxedCommand,
            ) -> Result<SandboxedCommand, nemo_sandbox::SandboxError> {
                Ok(c.clone())
            }
        }
        let mut p = nemo_terminal::SandboxPolicy::default();
        assert!(sandbox_preflight(&NoLoopback, Some(&p)).is_ok());
        p.network = NetworkPolicy::Loopback;
        let e = sandbox_preflight(&NoLoopback, Some(&p)).unwrap_err();
        assert!(
            e.contains("--network loopback") && e.contains("nemo doctor"),
            "{e}"
        );
    }

    #[test]
    fn preflight_refuses_a_backend_that_cannot_confine() {
        let e = sandbox_preflight(&nemo_sandbox::NullSandbox, None).unwrap_err();
        assert!(
            e.contains("nemo doctor") && e.contains("no OS sandbox"),
            "{e}"
        );
    }
}
