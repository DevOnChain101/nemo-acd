//! MCP servers for a run: which configs to load, which may start, and the tool adapter.
//!
//! Sources (later wins, Node `buildMcpServers` order minus Claude Code's files, which Rust does
//! not read): user `mcpServers` of nemo.json, project `.mcp.json` (gated), user `projects.<path>`
//! local scope, user `mcp`, project `.nemo/config.json` `mcp`/`mcpServers` (gated).
//! SECURITY: a project-scope server is repository content that would run a command. It starts
//! only with `--trust-project` AND an explicit approval (`--approve-mcp NAME`, or the USER's
//! settings `enabledMcpjsonServers` / `enableAllProjectMcpServers`; a project settings file
//! cannot approve itself) and never when the user settings disable it. Everything else is
//! reported as skipped and never spawned. Servers are not OS-sandboxed.
//! APPROVAL BINDING: an approval is tied to a hash of the definition (name, transport, command,
//! args, url, env, headers; see [`definition_hash`]). `--approve-mcp NAME` approves the definition
//! as it is NOW for this run only and prints it with its hash. A persisted approval must be
//! `NAME@HASH` (CLI or user `enabledMcpjsonServers`); if `.mcp.json` changes, the hash no longer
//! matches and the server is not started. A bare `NAME` in settings and
//! `enableAllProjectMcpServers` no longer approve project servers (they would survive an edit).
//! `${VAR}` in env/url/headers reads `Env::child_env` plus the names in the USER's
//! `nemo.mcp.env_passthrough` (user settings.json only), and the latter only for user-owned
//! scopes; see `nemo_mcp::McpOptions::env_passthrough`.

use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use nemo_config::{Config, Env};
use nemo_core::ToolContext;
use nemo_harness::sanitize_for_display as san;
use nemo_mcp::{
    McpContent, McpManager, McpOptions, McpServerConfig, ServerState, parse_claude_servers,
    parse_nemo_mcp_servers,
};
use nemo_settings::{ProjectTrust, Settings};
use nemo_tools::{
    Block, BoxFuture, Content, ImageSource, SideEffectClass, Tool, ToolOutput, ToolRuntime,
    ToolSpec,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Default)]
pub struct Plan {
    pub servers: Vec<McpServerConfig>,
    /// `(server, reason)`: configured but not started. Raw text from repository files: sanitize
    /// before printing.
    pub skipped: Vec<(String, String)>,
    /// One line per project server approved for this run, with its exact definition. Raw too.
    pub notices: Vec<String>,
    /// `nemo.mcp.env_passthrough` of the user settings file (see [`read_passthrough`]).
    pub env_passthrough: Vec<String>,
}

/// Hex prefix length shown and the shortest prefix accepted in `NAME@HASH`.
const HASH_SHOWN: usize = 24;
const HASH_MIN: usize = HASH_SHOWN;

/// SHA-256 over the whole definition (not the scope), so any edit changes it.
pub fn definition_hash(s: &McpServerConfig) -> String {
    let def = json!([
        s.name,
        s.transport.map(|t| t.as_str()),
        s.command,
        s.args,
        s.url,
        s.env,
        s.headers
    ]);
    Sha256::digest(def.to_string().as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// What approving this server means, exactly: command, args, url and the NAMES of env/headers.
pub fn describe(s: &McpServerConfig) -> String {
    let mut t = format!("{:?}", s.transport.map_or("?", |t| t.as_str()));
    if let Some(c) = &s.command {
        t += &format!(" command={c:?} args={:?}", s.args);
    }
    if let Some(u) = &s.url {
        t += &format!(" url={u:?}");
    }
    t += &format!(
        " env_keys={:?} headers={:?} hash={}",
        s.env.keys().collect::<Vec<_>>(),
        s.headers
            .iter()
            .map(|(k, v)| format!("{k}: {}…", v.chars().take(2).collect::<String>()))
            .collect::<Vec<_>>(),
        &definition_hash(s)[..HASH_SHOWN]
    );
    t
}

enum Appr {
    No,
    Yes,
    Stale(&'static str),
}

/// `entry` is `NAME` (only when `bare_ok`: a per-run CLI flag) or `NAME@HASHPREFIX`.
fn check(entry: &str, name: &str, hash: &str, bare_ok: bool) -> Appr {
    match entry.split_once('@') {
        None if entry == name && bare_ok => Appr::Yes,
        None if entry == name => {
            Appr::Stale("a bare name in settings is not bound to a definition")
        }
        Some((n, h)) if n == name => {
            if h.len() >= HASH_MIN && hash.starts_with(&h.to_ascii_lowercase()) {
                Appr::Yes
            } else {
                Appr::Stale("definition does not match the approved hash (changed since approval?)")
            }
        }
        _ => Appr::No,
    }
}

/// `nemo.mcp.env_passthrough` from the USER settings file only (a repository must not widen it).
pub fn read_passthrough(user_settings: &Path) -> Vec<String> {
    read_obj(user_settings)
        .and_then(|v| v.pointer("/nemo/mcp/env_passthrough").cloned())
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

fn read_obj(path: &Path) -> Option<Value> {
    let m = std::fs::metadata(path).ok()?;
    if !m.is_file() || m.len() > 1 << 20 {
        return None;
    }
    let mut s = String::new();
    std::fs::File::open(path)
        .ok()?
        .read_to_string(&mut s)
        .ok()?;
    serde_json::from_str(&s).ok().filter(Value::is_object)
}

fn put(all: &mut Vec<McpServerConfig>, add: Vec<McpServerConfig>) {
    for s in add {
        all.retain(|x| x.name != s.name);
        all.push(s);
    }
}

/// `user_cfg` must be loaded with `ProjectTrust::Untrusted` (no project layer in it).
pub fn plan(
    user_cfg: &Config,
    settings: &Settings,
    user_settings: &Settings,
    cwd: &Path,
    trust: ProjectTrust,
    approve: &[String],
) -> Plan {
    let x = &user_cfg.extra;
    let mut all: Vec<McpServerConfig> = vec![];
    let mut project: Vec<McpServerConfig> = vec![];

    let mut skipped = vec![];
    let mut add = |dst: &mut Vec<McpServerConfig>, (ok, bad): nemo_mcp::Parsed| {
        put(dst, ok);
        skipped.extend(bad.into_iter().map(|e| ("(invalid name)".to_string(), e)));
    };
    if let Some(m) = x.get("mcpServers") {
        add(&mut all, parse_claude_servers(m, Some("user")));
    }
    let root = &settings.project_root;
    if let Some(j) = read_obj(&root.join(".mcp.json")) {
        add(
            &mut project,
            parse_claude_servers(&j["mcpServers"], Some("project")),
        );
    }
    if let Some(local) = x
        .get("projects")
        .and_then(Value::as_object)
        .and_then(|p| {
            p.iter()
                .find(|(k, _)| Path::new(k) == cwd || Path::new(k) == root)
        })
        .and_then(|(_, e)| e.get("mcpServers"))
    {
        add(&mut all, parse_claude_servers(local, Some("local")));
    }
    if let Some(m) = x.get("mcp") {
        add(&mut all, parse_nemo_mcp_servers(m, Some("nemo")));
    }
    if let Some(c) = read_obj(&root.join(".nemo").join("config.json")) {
        for k in ["mcp", "mcpServers"] {
            if let Some(m) = c.get(k) {
                add(&mut project, parse_nemo_mcp_servers(m, Some("project")));
            }
        }
    }

    let mut notices = vec![];
    let ua = &user_settings.mcp_approval;
    let disabled = &settings.mcp_approval.disabled_mcpjson_servers;
    let mut started = vec![];
    for s in project {
        let hash = definition_hash(&s);
        let appr = approve
            .iter()
            .map(|e| check(e, &s.name, &hash, true))
            .chain(
                ua.enabled_mcpjson_servers
                    .iter()
                    .map(|e| check(e, &s.name, &hash, false)),
            )
            .fold(Appr::No, |a, b| match (a, b) {
                (Appr::Yes, _) | (_, Appr::Yes) => Appr::Yes,
                (Appr::No, b) => b,
                (a, _) => a,
            });
        let why = if disabled.contains(&s.name) {
            Some("disabled in settings".to_string())
        } else if trust != ProjectTrust::Trusted {
            Some(format!(
                "project not trusted (needs --trust-project and approval); definition: {}",
                describe(&s)
            ))
        } else {
            match appr {
                Appr::Yes => None,
                Appr::No => Some(format!(
                    "not approved: review it, then pass --approve-mcp {}@{} (or list that in your user settings enabledMcpjsonServers); definition: {}",
                    s.name,
                    &hash[..HASH_SHOWN],
                    describe(&s)
                )),
                Appr::Stale(w) => Some(format!(
                    "approval not applied: {w}; to approve the current definition pass --approve-mcp {}@{}; definition: {}",
                    s.name,
                    &hash[..HASH_SHOWN],
                    describe(&s)
                )),
            }
        };
        match why {
            Some(w) => skipped.push((s.name, w)),
            None => {
                notices.push(format!(
                    "MCP server \"{}\" ({}) approved: {}",
                    s.name,
                    s.scope.as_deref().unwrap_or("?"),
                    describe(&s)
                ));
                started.push(s);
            }
        }
    }
    // Project entries sit between user and local scope in Node; a user-owned entry of the same
    // name must win over an approved project one, so project ones are merged first.
    let mut servers = vec![];
    put(&mut servers, started);
    put(&mut servers, all);
    Plan {
        servers,
        skipped,
        notices,
        env_passthrough: vec![],
    }
}

/// Starts the servers and returns the tools of the connected ones plus value-free warnings.
pub async fn start(
    plan: &Plan,
    env: &Env,
    cwd: &Path,
) -> (Arc<McpManager>, Vec<Arc<dyn Tool>>, Vec<String>) {
    let mgr = Arc::new(McpManager::new(
        plan.servers.clone(),
        McpOptions {
            cwd: cwd.into(),
            env: env.clone(),
            env_passthrough: plan.env_passthrough.clone(),
            ..McpOptions::default()
        },
    ));
    mgr.start().await;
    let warnings = mgr
        .status()
        .into_iter()
        .filter(|s| s.state == ServerState::Failed)
        .map(|s| {
            format!(
                "MCP server \"{}\" failed to start: {}",
                san(&s.name),
                san(&s.error.unwrap_or_default())
            )
        })
        .collect();
    let tools = mgr
        .tools()
        .into_iter()
        .map(|t| -> Arc<dyn Tool> {
            Arc::new(McpTool {
                spec: ToolSpec {
                    name: t.name,
                    description: t.description,
                    input_schema: t.input_schema,
                    side_effect: SideEffectClass::External,
                    runtime: ToolRuntime::Mcp { server: t.server },
                },
                mgr: mgr.clone(),
            })
        })
        .collect();
    (mgr, tools, warnings)
}

/// One MCP tool. The permission engine sees the wire name `mcp__server__tool`; the harness
/// labels every tool result as untrusted data before the model reads it.
struct McpTool {
    spec: ToolSpec,
    mgr: Arc<McpManager>,
}

impl Tool for McpTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn execute<'a>(&'a self, input: Value, ctx: &'a ToolContext) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            let r = match self
                .mgr
                .call_wire(&self.spec.name, input, &ctx.cancel)
                .await
            {
                Ok(r) => r,
                Err(_) => return ToolOutput::soft_error("MCP call cancelled"),
            };
            let content = if r.content.iter().all(|c| matches!(c, McpContent::Text(_))) {
                Content::Text(r.text())
            } else {
                Content::Blocks(
                    r.content
                        .iter()
                        .map(|c| match c {
                            McpContent::Text(text) => Block::Text { text: text.clone() },
                            McpContent::Image { media_type, data } => Block::Image {
                                source: ImageSource::Base64 {
                                    media_type: media_type.clone(),
                                    data: data.clone(),
                                },
                            },
                        })
                        .collect(),
                )
            };
            let mut out = ToolOutput::text("");
            out.content = content;
            out.is_error = r.is_error;
            out.with_meta(json!({"mcp": true}))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nemo_config::Config;

    fn cfg(extra: Value) -> Config {
        Config {
            extra: extra.as_object().unwrap().clone(),
            ..Config::default()
        }
    }

    #[test]
    fn project_servers_need_trust_and_user_approval() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(
            d.path().join(".mcp.json"),
            json!({"mcpServers":{"repo":{"command":"x"},"other":{"command":"y"}}}).to_string(),
        )
        .unwrap();
        let user = cfg(json!({"mcpServers":{"mine":{"command":"z"}}}));
        let s = Settings {
            project_root: d.path().to_path_buf(),
            ..Settings::default()
        };
        let names = |p: &Plan| p.servers.iter().map(|s| s.name.clone()).collect::<Vec<_>>();

        let p = plan(
            &user,
            &s,
            &s,
            d.path(),
            ProjectTrust::Untrusted,
            &["repo".into()],
        );
        assert_eq!(names(&p), ["mine"]);
        assert_eq!(p.skipped.len(), 2);
        assert!(p.skipped[0].1.contains("not trusted"));

        let p = plan(&user, &s, &s, d.path(), ProjectTrust::Trusted, &[]);
        assert_eq!(names(&p), ["mine"], "trusted but not approved");
        assert!(p.skipped[0].1.contains("not approved"));

        let p = plan(
            &user,
            &s,
            &s,
            d.path(),
            ProjectTrust::Trusted,
            &["repo".into()],
        );
        assert_eq!(names(&p), ["repo", "mine"]);

        // a project settings file cannot approve itself: only the user-settings view counts
        let mut proj_only = s.clone();
        proj_only.mcp_approval.enable_all_project_mcp_servers = true;
        let p = plan(&user, &proj_only, &s, d.path(), ProjectTrust::Trusted, &[]);
        assert_eq!(names(&p), ["mine"]);
        // enableAll in the user settings no longer approves (it would survive a definition edit)
        let p = plan(&user, &s, &proj_only, d.path(), ProjectTrust::Trusted, &[]);
        assert_eq!(names(&p), ["mine"]);

        let mut off = s.clone();
        off.mcp_approval.disabled_mcpjson_servers = vec!["repo".into()];
        let p = plan(
            &user,
            &off,
            &s,
            d.path(),
            ProjectTrust::Trusted,
            &["repo".into()],
        );
        assert_eq!(names(&p), ["mine"]);
    }

    fn repo(mcp_json: Value) -> (tempfile::TempDir, Settings) {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(".mcp.json"), mcp_json.to_string()).unwrap();
        let s = Settings {
            project_root: d.path().to_path_buf(),
            ..Settings::default()
        };
        (d, s)
    }

    fn started(p: &Plan) -> Vec<&str> {
        p.servers.iter().map(|s| s.name.as_str()).collect()
    }

    #[test]
    fn approval_is_bound_to_the_definition_hash() {
        let (d, s) =
            repo(json!({"mcpServers":{"repo":{"command":"x","args":["a"],"env":{"K":"v"}}}}));
        let user = cfg(json!({}));
        let run =
            |ua: &Settings, ap: &[String]| plan(&user, &s, ua, d.path(), ProjectTrust::Trusted, ap);
        let first = run(&s, &[]);
        let why = &first.skipped[0].1;
        let hash = definition_hash(&the_server(&d));
        // the skip reason shows the exact command, args and env KEYS (never values) and the hash
        assert!(
            why.contains("command=\"x\"") && why.contains("env_keys=[\"K\"]"),
            "{why}"
        );
        assert!(
            why.contains(&hash[..HASH_SHOWN]) && !why.contains("\"v\""),
            "{why}"
        );

        let good = format!("repo@{}", &hash[..HASH_SHOWN]);
        let p = run(&s, std::slice::from_ref(&good));
        assert_eq!(started(&p), ["repo"]);
        assert!(p.notices[0].contains("command=\"x\""), "{:?}", p.notices);
        // persisted in the USER settings: only NAME@HASH counts, a bare name does not
        let mut ua = s.clone();
        ua.mcp_approval.enabled_mcpjson_servers = vec![good];
        assert_eq!(started(&run(&ua, &[])), ["repo"]);
        ua.mcp_approval.enabled_mcpjson_servers = vec!["repo".into()];
        assert!(started(&run(&ua, &[])).is_empty());

        // .mcp.json edited after the approval: not started
        let mut ua = s.clone();
        ua.mcp_approval.enabled_mcpjson_servers = vec![format!("repo@{}", &hash[..HASH_SHOWN])];
        for edit in [
            json!({"command":"evil","args":["a"],"env":{"K":"v"}}),
            json!({"command":"x","args":["a","b"],"env":{"K":"v"}}),
            json!({"command":"x","args":["a"],"env":{"K":"w"}}),
            json!({"command":"x","args":["a"],"env":{"K":"v"},"headers":{"H":"1"}}),
        ] {
            std::fs::write(
                d.path().join(".mcp.json"),
                json!({"mcpServers":{"repo":edit}}).to_string(),
            )
            .unwrap();
            let p = run(&ua, &[]);
            assert!(started(&p).is_empty(), "{edit}");
            assert!(p.skipped[0].1.contains("does not match"), "{:?}", p.skipped);
            let p = run(&s, &[format!("repo@{}", &hash[..HASH_SHOWN])]);
            assert!(started(&p).is_empty(), "{edit}");
        }
        // a hash prefix shorter than the shown one never matches
        for n in [4, 16, HASH_SHOWN - 1] {
            assert!(
                started(&run(&s, &[format!("repo@{}", &hash[..n])])).is_empty(),
                "{n}"
            );
        }
    }

    #[test]
    fn describe_masks_header_values() {
        let mut c = McpServerConfig::http("h", "http://example.test/");
        c.headers
            .insert("Authorization".into(), "Bearer SECRET".into());
        let d = describe(&c);
        assert!(
            d.contains("Authorization: Be…") && !d.contains("SECRET") && !d.contains("Bearer"),
            "{d}"
        );
    }

    pub(super) fn the_server(d: &tempfile::TempDir) -> McpServerConfig {
        let j = read_obj(&d.path().join(".mcp.json")).unwrap();
        parse_claude_servers(&j["mcpServers"], Some("project"))
            .0
            .remove(0)
    }

    #[test]
    fn hostile_server_names_are_rejected_not_started() {
        let (d, s) = repo(
            json!({"mcpServers":{"a\u{1b}]52;;QQ==\u{7}":{"command":"x"},"ok":{"command":"y"}}}),
        );
        let user = cfg(json!({}));
        let p = plan(
            &user,
            &s,
            &s,
            d.path(),
            ProjectTrust::Trusted,
            &["ok".into(), "a\u{1b}]52;;QQ==\u{7}".into()],
        );
        assert_eq!(started(&p), ["ok"]);
        assert!(
            p.skipped
                .iter()
                .any(|(_, w)| w.contains("invalid MCP server name")),
            "{:?}",
            p.skipped
        );
        // what the CLI prints is free of escape sequences
        let line = format!("{}: {}", p.skipped[0].0, san(&p.skipped[0].1));
        assert!(!line.contains(['\x1b', '\x07']), "{line:?}");
    }

    #[test]
    fn passthrough_comes_from_user_settings_file() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("settings.json");
        std::fs::write(
            &f,
            json!({"nemo":{"mcp":{"env_passthrough":["A","B"]}}}).to_string(),
        )
        .unwrap();
        assert_eq!(read_passthrough(&f), ["A", "B"]);
        assert!(read_passthrough(&d.path().join("none.json")).is_empty());
    }
}
