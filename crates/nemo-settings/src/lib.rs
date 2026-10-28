//! `settings.json` loader (port of Node `claudecfg.js` `mergeSettings`, `loadNemoEnv`,
//! `loadAutoModeConfig`, `loadBlockOutsideReads`, `findProjectRoot`; docs/specs/config.md).
//!
//! Layers, lowest to highest: `<nemo_home>/settings.json` (user), `<project_root>/.nemo/settings.json`
//! (project). Lists are unions (deduped, order kept); scalars: last layer wins. Unknown keys are
//! ignored, malformed files give a diagnostic and are skipped. Node has no third "local" layer.
//!
//! SECURITY (deviation from Node): the project file comes from a possibly hostile repository.
//! Unless the caller passes [`ProjectTrust::Trusted`], it can only TIGHTEN: `deny`/`ask` rules, the
//! modes `default|manual|plan|dontAsk` and `mcp` disables are kept; `allow` rules, hooks,
//! `additionalDirectories`, `disableAllHooks`, `env`, MCP approvals and the `nemo` section are
//! dropped with a diagnostic (counts and key names only, never values). `auto` and
//! `bypassPermissions` are refused from the project even when trusted (as Node).
//!
//! Not ported: Claude Code's `~/.claude/settings.json` layer (needs `CLAUDE_CONFIG_DIR`, which Rust
//! does not read; config.md §4), `source` tags on hook entries.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use nemo_hooks::{HookGroup, HooksConfig};
use serde_json::{Map, Value};

pub use nemo_config::{LoadContext, ProjectTrust};

/// Highest `nemo.version` this build understands; a newer section is ignored with a diagnostic.
pub const NEMO_SECTION_VERSION: u64 = 1;
const MAX_FILE_BYTES: u64 = 1 << 20;
/// Modes an untrusted project may select: none of them allows more than the default.
const UNTRUSTED_PROJECT_MODES: [&str; 4] = ["default", "manual", "plan", "dontAsk"];
const PROJECT_FORBIDDEN_MODES: [&str; 2] = ["auto", "bypassPermissions"];

/// Plain permission inputs; the harness converts these to its permission engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRules {
    pub allow: Vec<String>,
    pub ask: Vec<String>,
    pub deny: Vec<String>,
    pub additional_directories: Vec<String>,
    /// Raw `defaultMode` string, `"default"` when unset (mode names are interpreted downstream).
    pub default_mode: String,
    /// A project `defaultMode` that was refused (Node `ignoredDefaultMode`).
    pub ignored_default_mode: Option<String>,
    /// User-only.
    pub block_reads_outside_working_directories: bool,
}

impl Default for PermissionRules {
    fn default() -> Self {
        Self {
            allow: vec![],
            ask: vec![],
            deny: vec![],
            additional_directories: vec![],
            default_mode: "default".into(),
            ignored_default_mode: None,
            block_reads_outside_working_directories: false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct McpApproval {
    pub enabled_mcpjson_servers: Vec<String>,
    pub disabled_mcpjson_servers: Vec<String>,
    pub enable_all_project_mcp_servers: bool,
}

/// User-only (`autoMode`): steers the classifier that approves without asking.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AutoModeConfig {
    pub environment: Option<Vec<String>>,
    pub allow: Option<Vec<String>>,
    pub soft_deny: Option<Vec<String>>,
    pub hard_deny: Option<Vec<String>>,
    pub classify_all_shell: Option<bool>,
    pub use_auto_mode_during_plan: Option<bool>,
    /// Model that judges auto-mode calls (a model of the session's provider); `None` = the session model.
    pub model: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Budget {
    pub max_turns: Option<u64>,
    pub max_tool_calls: Option<u64>,
    pub max_verify_attempts: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Compat {
    pub write_node_transcripts: Option<bool>,
}

/// The Rust-only top-level `"nemo"` object (config.md §6). Every field optional.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NemoSettings {
    pub budget: Budget,
    /// Verification policy keys, shallow-merged per key; interpreted by `nemo-verification`.
    pub verification: Map<String, Value>,
    pub compat: Compat,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Settings {
    pub project_root: PathBuf,
    pub permissions: PermissionRules,
    pub hooks: HooksConfig,
    pub disable_all_hooks: bool,
    pub mcp_approval: McpApproval,
    /// settings.json `env` block (string/number/bool values), secret-shaped names refused.
    pub env: BTreeMap<String, String>,
    pub env_refused: Vec<String>,
    pub auto_mode: AutoModeConfig,
    pub nemo: NemoSettings,
    /// Non-secret notes: malformed files, ignored/refused items. Never contains setting values.
    pub diagnostics: Vec<String>,
}

/// Nearest ancestor of `cwd` (inclusive) holding `.nemo` or `.git`, else `cwd`. Pure lexical walk
/// from the given path; no process cwd, no canonicalisation.
pub fn find_project_root(cwd: &Path) -> PathBuf {
    cwd.ancestors()
        .filter(|d| !d.as_os_str().is_empty())
        .find(|d| d.join(".nemo").exists() || d.join(".git").exists())
        .unwrap_or(cwd)
        .to_path_buf()
}

/// Node `isSecretEnvKey`: secret-looking substring or provider-ish prefix.
pub fn is_secret_env_key(key: &str) -> bool {
    let up = key.to_ascii_uppercase();
    [
        "KEY",
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "CREDENTIAL",
        "AUTH",
        "COOKIE",
        "SESSION",
    ]
    .iter()
    .any(|s| up.contains(s))
        || [
            "ANTHROPIC_",
            "CLAUDE_CODE_",
            "OPENAI_",
            "AWS_",
            "AZURE_",
            "GOOGLE_",
            "VERTEX_",
            "NEMO_",
        ]
        .iter()
        .any(|p| key.starts_with(p))
}

pub fn load_settings(ctx: &LoadContext<'_>, trust: ProjectTrust) -> Settings {
    let project_root = find_project_root(ctx.cwd);
    let mut s = Settings {
        project_root: project_root.clone(),
        ..Settings::default()
    };
    let user = ctx.nemo_home.join("settings.json");
    let project = project_root.join(".nemo").join("settings.json");
    if let Some(raw) = read_json(&user, "user settings", &mut s.diagnostics) {
        s.apply(&raw, true, true);
    }
    if let Some(raw) = read_json(&project, "project settings", &mut s.diagnostics) {
        s.apply(&raw, false, trust == ProjectTrust::Trusted);
    }
    s
}

/// Missing file: silent. Unreadable, oversized, not a regular file or malformed: diagnostic.
fn read_json(path: &Path, what: &str, diag: &mut Vec<String>) -> Option<Value> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        diag.push(format!(
            "{what}: not a regular file or larger than 1 MiB, skipped"
        ));
        return None;
    }
    let mut text = String::new();
    if std::fs::File::open(path)
        .and_then(|f| f.take(MAX_FILE_BYTES).read_to_string(&mut text))
        .is_err()
    {
        diag.push(format!("{what}: unreadable, skipped"));
        return None;
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(v @ Value::Object(_)) => Some(v),
        Ok(_) => None,
        Err(e) => {
            diag.push(format!("malformed JSON in {what}: {e}"));
            None
        }
    }
}

fn strings(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn union(into: &mut Vec<String>, add: Vec<String>) {
    for x in add {
        if !into.contains(&x) {
            into.push(x);
        }
    }
}

impl Settings {
    /// `user`: the user file. `trusted`: user file, or project file with `Trusted`.
    fn apply(&mut self, raw: &Value, user: bool, trusted: bool) {
        let what = if user {
            "user settings"
        } else {
            "project settings"
        };
        let perm = raw.get("permissions").filter(|p| p.is_object());
        let list = |k| strings(perm.and_then(|p| p.get(k)));
        let p = &mut self.permissions;
        union(&mut p.deny, list("deny"));
        union(&mut p.ask, list("ask"));
        let (allow, dirs) = (list("allow"), list("additionalDirectories"));
        if trusted {
            union(&mut p.allow, allow);
            p.additional_directories.extend(dirs);
        } else if !allow.is_empty() || !dirs.is_empty() {
            self.diagnostics.push(format!(
                "{what}: ignored {} allow rule(s) and {} additionalDirectories (project not trusted)",
                allow.len(),
                dirs.len()
            ));
        }
        if let Some(mode) = perm
            .and_then(|p| p.get("defaultMode"))
            .and_then(Value::as_str)
        {
            let refused = !user
                && (PROJECT_FORBIDDEN_MODES.contains(&mode)
                    || (!trusted && !UNTRUSTED_PROJECT_MODES.contains(&mode)));
            if refused {
                p.ignored_default_mode = Some(mode.into());
                self.diagnostics.push(format!(
                    "{what}: defaultMode \"{mode}\" is not allowed from a project, ignored"
                ));
            } else {
                p.default_mode = mode.into();
            }
        }
        if user {
            if let Some(b) = perm
                .and_then(|p| p.get("blockReadsOutsideWorkingDirectories"))
                .map(|b| b == &Value::Bool(true))
            {
                p.block_reads_outside_working_directories = b;
            }
            self.apply_auto_mode(raw);
        }

        if let Some(hooks) = raw.get("hooks").and_then(Value::as_object) {
            self.apply_hooks(hooks, trusted, what);
        }
        if trusted {
            if let Some(b) = raw.get("disableAllHooks").and_then(Value::as_bool) {
                self.disable_all_hooks = b;
            }
            self.apply_env(raw);
            self.apply_nemo(raw, what);
        } else if raw.get("disableAllHooks").is_some()
            || raw.get("env").is_some()
            || raw.get("nemo").is_some()
        {
            self.diagnostics.push(format!(
                "{what}: ignored disableAllHooks/env/nemo keys (project not trusted)"
            ));
        }

        let m = &mut self.mcp_approval;
        union(
            &mut m.disabled_mcpjson_servers,
            strings(raw.get("disabledMcpjsonServers")),
        );
        if trusted {
            union(
                &mut m.enabled_mcpjson_servers,
                strings(raw.get("enabledMcpjsonServers")),
            );
            if let Some(b) = raw
                .get("enableAllProjectMcpServers")
                .and_then(Value::as_bool)
            {
                m.enable_all_project_mcp_servers = b;
            }
        } else if raw.get("enabledMcpjsonServers").is_some()
            || raw.get("enableAllProjectMcpServers").is_some()
        {
            self.diagnostics.push(format!(
                "{what}: ignored MCP approvals (project not trusted)"
            ));
        }
    }

    fn apply_hooks(&mut self, hooks: &Map<String, Value>, trusted: bool, what: &str) {
        let mut dropped = 0usize;
        for (event, entries) in hooks {
            let Some(entries) = entries.as_array() else {
                continue;
            };
            for entry in entries {
                if !trusted {
                    dropped += 1;
                } else if let Ok(g) = serde_json::from_value::<HookGroup>(entry.clone()) {
                    self.hooks.entry(event.clone()).or_default().push(g);
                } else {
                    self.diagnostics.push(format!(
                        "{what}: a hook group has an invalid shape, skipped"
                    ));
                }
            }
        }
        if dropped > 0 {
            self.diagnostics.push(format!(
                "{what}: ignored {dropped} hook group(s) (project not trusted)"
            ));
        }
    }

    fn apply_env(&mut self, raw: &Value) {
        let Some(env) = raw.get("env").and_then(Value::as_object) else {
            return;
        };
        for (k, v) in env {
            if is_secret_env_key(k) {
                if !self.env_refused.contains(k) {
                    self.env_refused.push(k.clone());
                }
                continue;
            }
            match v {
                Value::String(s) => self.env.insert(k.clone(), s.clone()),
                Value::Number(_) | Value::Bool(_) => self.env.insert(k.clone(), v.to_string()),
                _ => None,
            };
        }
    }

    fn apply_auto_mode(&mut self, raw: &Value) {
        let Some(am) = raw.get("autoMode").and_then(Value::as_object) else {
            return;
        };
        let list = |k| am.get(k).filter(|v| v.is_array()).map(|v| strings(Some(v)));
        let flag = |k| am.get(k).and_then(Value::as_bool);
        self.auto_mode = AutoModeConfig {
            environment: list("environment"),
            allow: list("allow"),
            soft_deny: list("soft_deny"),
            hard_deny: list("hard_deny"),
            classify_all_shell: flag("classifyAllShell"),
            use_auto_mode_during_plan: flag("useAutoModeDuringPlan"),
            model: am
                .get("model")
                .and_then(Value::as_str)
                .filter(|m| !m.trim().is_empty())
                .map(|m| m.trim().to_string()),
        };
    }

    fn apply_nemo(&mut self, raw: &Value, what: &str) {
        let Some(n) = raw.get("nemo").and_then(Value::as_object) else {
            return;
        };
        let version = n.get("version").and_then(Value::as_u64).unwrap_or(1);
        if version > NEMO_SECTION_VERSION {
            self.diagnostics.push(format!(
                "{what}: nemo section version {version} is newer than supported {NEMO_SECTION_VERSION}, ignored"
            ));
            return;
        }
        let num = |o: Option<&Value>, k: &str| o.and_then(|b| b.get(k)).and_then(Value::as_u64);
        let b = n.get("budget").filter(|b| b.is_object());
        let t = &mut self.nemo.budget;
        t.max_turns = num(b, "max_turns").or(t.max_turns);
        t.max_tool_calls = num(b, "max_tool_calls").or(t.max_tool_calls);
        t.max_verify_attempts = num(b, "max_verify_attempts").or(t.max_verify_attempts);
        if let Some(v) = n.get("verification").and_then(Value::as_object) {
            self.nemo.verification.extend(v.clone());
        }
        if let Some(w) = n
            .get("compat")
            .and_then(|c| c.get("write_node_transcripts"))
            .and_then(Value::as_bool)
        {
            self.nemo.compat.write_node_transcripts = Some(w);
        }
    }
}
