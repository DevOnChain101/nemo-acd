//! MCP client manager (port of mcp.js `McpManager`). Never panics or errors out of its public
//! methods: failures land in `status()` as `Failed` with a short, secret-free message.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use nemo_config::Env;
use process_wrap::tokio::CommandWrap;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, ClientCapabilities, ClientConfig, Implementation,
};
use rmcp::service::{NotificationContext, RunningService};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{ClientHandler, Peer, RoleClient, ServiceExt};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::config::{McpServerConfig, TransportKind, expand_vars, stdio_base_env};
use crate::naming::{parse_tool_name_fallback, tool_name_for};
use crate::result::{McpToolResult, normalize_result, truncate};

const MAX_DESCRIPTION_LEN: usize = 1000;
const MAX_STDERR_LINES: usize = 50;
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

pub const SSE_UNSUPPORTED: &str = "transport \"sse\" (legacy HTTP+SSE) is not supported by this build; use type \"http\" (streamable HTTP) or stdio";

#[derive(Debug, Clone)]
pub struct McpOptions {
    /// Working directory for stdio servers.
    pub cwd: PathBuf,
    /// Injected environment: source of the stdio base env and (through an allowlist, see
    /// `env_passthrough`) of `${VAR}` expansion.
    pub env: Env,
    /// Names the USER listed (`nemo.mcp.env_passthrough`) as expandable in `${VAR}` besides
    /// `Env::child_env`. Honoured only for user-owned scopes ("user", "nemo"); a project or
    /// local-scope definition can only ever expand `child_env` names.
    pub env_passthrough: Vec<String>,
    pub client_name: String,
    pub client_version: String,
    /// Covers spawn + initialize + first tool/resource/prompt listing.
    pub connect_timeout: Duration,
    pub tool_timeout: Duration,
}

impl Default for McpOptions {
    fn default() -> Self {
        Self {
            cwd: PathBuf::from("."),
            env: Env::default(),
            env_passthrough: vec![],
            client_name: "nemo".into(),
            client_version: env!("CARGO_PKG_VERSION").into(),
            connect_timeout: Duration::from_secs(30),
            tool_timeout: Duration::from_secs(120),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerState {
    Connecting,
    Connected,
    Failed,
    Disabled,
    /// After `close()` / `reconnect()` teardown.
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerStatus {
    pub name: String,
    pub scope: Option<String>,
    pub transport: &'static str,
    pub state: ServerState,
    pub tool_count: usize,
    pub resource_count: usize,
    pub prompt_count: usize,
    pub error: Option<String>,
    pub server_info: Option<(String, String)>,
}

/// A discovered tool in the agent registry shape.
#[derive(Debug, Clone, PartialEq)]
pub struct McpTool {
    /// Provider-safe wire name `mcp__<server>__<tool>`.
    pub name: String,
    pub server: String,
    pub tool: String,
    pub description: String,
    pub input_schema: Value,
}

/// The call was cancelled by the caller's token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Aborted;

#[derive(Clone)]
struct ToolDef {
    name: String,
    description: Option<String>,
    input_schema: Value,
}

struct Conn {
    service: RunningService<RoleClient, Handler>,
    peer: Peer<RoleClient>,
    pid: Option<u32>,
}

struct Entry {
    cfg: McpServerConfig,
    state: ServerState,
    error: Option<String>,
    conn: Option<Conn>,
    /// Bumped on every connect attempt; stale background tasks compare it before writing.
    generation: u64,
    kind: Option<TransportKind>,
    tools: Vec<ToolDef>,
    resource_count: usize,
    prompt_count: usize,
    server_info: Option<(String, String)>,
    stderr: VecDeque<String>,
}

struct Shared {
    opts: McpOptions,
    entries: Mutex<Vec<Entry>>,
    names: Mutex<HashMap<String, (String, String)>>,
    status_tx: watch::Sender<Vec<ServerStatus>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Entry {
    fn status(&self) -> ServerStatus {
        ServerStatus {
            name: self.cfg.name.clone(),
            scope: self.cfg.scope.clone(),
            transport: self
                .kind
                .or(self.cfg.transport)
                .unwrap_or(if self.cfg.url.is_some() {
                    TransportKind::Http
                } else {
                    TransportKind::Stdio
                })
                .as_str(),
            state: self.state,
            tool_count: self.tools.len(),
            resource_count: self.resource_count,
            prompt_count: self.prompt_count,
            error: self.error.clone(),
            server_info: self.server_info.clone(),
        }
    }

    fn reset(&mut self) {
        self.conn = None;
        self.tools.clear();
        self.resource_count = 0;
        self.prompt_count = 0;
        self.server_info = None;
    }
}

impl Shared {
    fn emit(&self) {
        let s = lock(&self.entries).iter().map(Entry::status).collect();
        self.status_tx.send_replace(s);
    }

    /// Runs `f` on the entry if it is still on connection `generation`.
    fn with<R>(&self, name: &str, generation: u64, f: impl FnOnce(&mut Entry) -> R) -> Option<R> {
        let mut g = lock(&self.entries);
        g.iter_mut()
            .find(|e| e.cfg.name == name && e.generation == generation)
            .map(f)
    }

    /// The only variables `${VAR}` in `cfg` may read: `child_env`, plus the user's passthrough
    /// names for user-owned scopes. Hostile repository config can never reach other secrets.
    fn expansion_env(&self, cfg: &McpServerConfig) -> Env {
        let base = self.opts.env.child_env();
        if !matches!(cfg.scope.as_deref(), Some("user" | "nemo")) {
            // proxy URLs may carry credentials and SSL_CERT_* paths are local: a repo-chosen
            // url/header must not be able to read them (children still inherit them as env)
            return Env::from_pairs(base.iter().filter(|(k, _)| {
                !k.to_ascii_lowercase().contains("proxy") && !k.starts_with("SSL_CERT")
            }));
        }
        let extra = self
            .opts
            .env_passthrough
            .iter()
            .filter_map(|n| Some((n.as_str(), self.opts.env.get(n)?)));
        Env::from_pairs(base.iter().chain(extra))
    }

    fn secrets(&self, cfg: &McpServerConfig) -> Vec<String> {
        let env = &self.expansion_env(cfg);
        let mut v: Vec<String> = cfg
            .env
            .values()
            .chain(cfg.headers.values())
            .chain(cfg.url.iter())
            .cloned()
            .collect();
        let expanded: Vec<String> = v.iter().filter_map(|s| expand_vars(s, env).ok()).collect();
        v.extend(expanded);
        v.retain(|s| s.len() >= 4);
        v
    }

    fn short_error(&self, cfg: &McpServerConfig, msg: &str) -> String {
        let mut m = msg.to_string();
        for s in self.secrets(cfg) {
            m = m.replace(&s, "***");
        }
        truncate(&m, 300)
    }

    async fn refresh_tools(
        &self,
        name: &str,
        generation: u64,
        peer: &Peer<RoleClient>,
    ) -> Result<(), String> {
        let tools = peer.list_all_tools().await.map_err(|e| e.to_string())?;
        let defs = tools
            .iter()
            .filter_map(|t| serde_json::to_value(t).ok())
            .map(|v| ToolDef {
                name: v["name"].as_str().unwrap_or("").to_string(),
                description: v["description"].as_str().map(str::to_string),
                input_schema: v
                    .get("inputSchema")
                    .cloned()
                    .unwrap_or_else(|| json!({"type":"object","properties":{}})),
            })
            .collect();
        self.with(name, generation, |e| e.tools = defs);
        Ok(())
    }

    /// Resource/prompt counts; a failing listing counts as 0 (mcp.js:385).
    async fn refresh_counts(&self, name: &str, generation: u64, peer: &Peer<RoleClient>) {
        let caps = peer.peer_info().map(|i| i.capabilities.clone());
        let Some(caps) = caps else { return };
        if caps.resources.is_some() {
            let n = peer.list_all_resources().await.map_or(0, |r| r.len());
            self.with(name, generation, |e| e.resource_count = n);
        }
        if caps.prompts.is_some() {
            let n = peer.list_all_prompts().await.map_or(0, |r| r.len());
            self.with(name, generation, |e| e.prompt_count = n);
        }
    }

    async fn connect_one(self: &Arc<Self>, name: &str) {
        let (cfg, generation) = {
            let mut g = lock(&self.entries);
            let Some(e) = g.iter_mut().find(|e| e.cfg.name == name) else {
                return;
            };
            e.generation += 1;
            e.error = None;
            if e.cfg.disabled {
                e.state = ServerState::Disabled;
                e.error = e.cfg.disabled_reason.clone();
                drop(g);
                self.emit();
                return;
            }
            e.state = ServerState::Connecting;
            (e.cfg.clone(), e.generation)
        };
        self.emit();
        let res =
            tokio::time::timeout(self.opts.connect_timeout, self.establish(&cfg, generation)).await;
        let res = res.unwrap_or_else(|_| {
            Err(format!(
                "connect timed out after {}ms",
                self.opts.connect_timeout.as_millis()
            ))
        });
        match res {
            Ok((conn, kind)) => {
                let info = conn.peer.peer_info().and_then(|i| {
                    i.server_info
                        .as_ref()
                        .map(|s| (s.name.clone(), s.version.clone()))
                });
                self.with(name, generation, |e| {
                    e.conn = Some(conn);
                    e.kind = Some(kind);
                    e.server_info = info;
                    e.state = ServerState::Connected;
                    e.error = None;
                });
            }
            Err(msg) => {
                let msg = self.short_error(&cfg, &msg);
                self.with(name, generation, |e| {
                    e.reset();
                    e.state = ServerState::Failed;
                    e.error = Some(msg);
                });
            }
        }
        self.emit();
    }

    async fn establish(
        self: &Arc<Self>,
        cfg: &McpServerConfig,
        generation: u64,
    ) -> Result<(Conn, TransportKind), String> {
        let mut errs: Vec<String> = Vec::new();
        for kind in cfg.transport_candidates() {
            if kind == TransportKind::Sse && !errs.is_empty() {
                let last = errs.pop().unwrap_or_default();
                errs.push(format!(
                    "{} (legacy SSE fallback not supported)",
                    truncate(&last, 200)
                ));
                continue;
            }
            match self.establish_one(cfg, generation, kind).await {
                Ok(conn) => return Ok((conn, kind)),
                Err(e) => errs.push(e),
            }
        }
        Err(errs.join("; "))
    }

    async fn establish_one(
        self: &Arc<Self>,
        cfg: &McpServerConfig,
        generation: u64,
        kind: TransportKind,
    ) -> Result<Conn, String> {
        let handler = Handler {
            shared: self.clone(),
            name: cfg.name.clone(),
            generation,
        };
        let (service, pid) = match kind {
            TransportKind::Stdio => {
                let (t, pid) = self.spawn_stdio(cfg, generation)?;
                (handler.serve(t).await.map_err(|e| e.to_string())?, pid)
            }
            TransportKind::Http => {
                let t = StreamableHttpClientTransport::from_config(self.http_config(cfg)?);
                (handler.serve(t).await.map_err(|e| e.to_string())?, None)
            }
            TransportKind::Sse => return Err(SSE_UNSUPPORTED.into()),
        };
        let peer = service.peer().clone();
        let mut conn = Conn {
            service,
            peer: peer.clone(),
            pid,
        };
        let listed = async {
            self.refresh_tools(&cfg.name, generation, &peer).await?;
            self.refresh_counts(&cfg.name, generation, &peer).await;
            Ok::<(), String>(())
        };
        if let Err(e) = listed.await {
            let _ = conn.service.close().await;
            return Err(e);
        }
        Ok(conn)
    }

    fn spawn_stdio(
        self: &Arc<Self>,
        cfg: &McpServerConfig,
        generation: u64,
    ) -> Result<(TokioChildProcess, Option<u32>), String> {
        let command = cfg
            .command
            .clone()
            .filter(|c| !c.is_empty())
            .ok_or("stdio server has no command")?;
        let mut env = stdio_base_env(&self.opts.env);
        let xenv = self.expansion_env(cfg);
        for (k, v) in &cfg.env {
            env.insert(k.clone(), expand(v, &xenv)?);
        }
        // Windows `CreateProcess` does not resolve `.cmd` shims such as `npx`; Node's cross-spawn did.
        let program = if cfg!(windows) {
            let path = env
                .get("PATH")
                .or_else(|| env.get("Path"))
                .cloned()
                .unwrap_or_default();
            which::which_in(&command, Some(path), &self.opts.cwd)
                .map_or(command.clone().into(), |p| p.into_os_string())
        } else {
            command.clone().into()
        };
        let mut wrap = CommandWrap::with_new(program, |c| {
            c.args(&cfg.args)
                .current_dir(&self.opts.cwd)
                .env_clear()
                .envs(&env);
        });
        #[cfg(unix)]
        wrap.wrap(process_wrap::tokio::ProcessGroup::leader());
        #[cfg(windows)]
        wrap.wrap(process_wrap::tokio::JobObject);
        let (proc, stderr) = TokioChildProcess::builder(wrap)
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to start \"{command}\": {e}"))?;
        if let Some(stderr) = stderr {
            let sh = self.clone();
            let name = cfg.name.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if line.is_empty() {
                        continue;
                    }
                    sh.with(&name, generation, |e| {
                        e.stderr.push_back(line);
                        if e.stderr.len() > MAX_STDERR_LINES {
                            e.stderr.pop_front();
                        }
                    });
                }
            });
        }
        let pid = proc.id();
        Ok((proc, pid))
    }

    fn http_config(
        &self,
        cfg: &McpServerConfig,
    ) -> Result<StreamableHttpClientTransportConfig, String> {
        let url = cfg
            .url
            .as_deref()
            .filter(|u| !u.is_empty())
            .ok_or("http server has no url")?;
        let xenv = self.expansion_env(cfg);
        let mut c = StreamableHttpClientTransportConfig::with_uri(expand(url, &xenv)?);
        for (k, v) in &cfg.headers {
            let v = expand(v, &xenv)?;
            if k.eq_ignore_ascii_case("authorization") {
                // rmcp reserves this header and sends `Authorization: Bearer <token>` itself.
                let token = v
                    .strip_prefix("Bearer ")
                    .or_else(|| v.strip_prefix("bearer "));
                c = c.auth_header(
                    token
                        .ok_or("only \"Authorization: Bearer <token>\" is supported")?
                        .to_string(),
                );
            } else {
                let name = http::HeaderName::from_bytes(k.as_bytes())
                    .map_err(|_| format!("invalid header name \"{k}\""))?;
                let val = http::HeaderValue::from_str(&v)
                    .map_err(|_| format!("invalid value for header \"{k}\""))?;
                c.custom_headers.insert(name, val);
            }
        }
        Ok(c)
    }

    async fn close_entry(&self, name: &str) {
        let conn = {
            let mut g = lock(&self.entries);
            let Some(e) = g.iter_mut().find(|e| e.cfg.name == name) else {
                return;
            };
            e.generation += 1;
            let conn = e.conn.take();
            e.reset();
            if matches!(e.state, ServerState::Connected | ServerState::Failed) {
                e.state = ServerState::Closed;
            }
            conn
        };
        if let Some(mut c) = conn {
            let _ = tokio::time::timeout(CLOSE_TIMEOUT, c.service.close()).await;
            // A server that exits on stdin EOF can leave grandchildren in its group; reap them.
            if let Some(pid) = c.pid {
                kill_group(pid);
            }
        }
    }
}

fn expand(v: &str, env: &Env) -> Result<String, String> {
    expand_vars(v, env).map_err(|n| {
        format!("variable \"{n}\" is not available for ${{{n}}} expansion (not in the child-env allowlist or the user's nemo.mcp.env_passthrough)")
    })
}

/// The child is its own process-group leader (pgid == pid). Best effort; no-op on Windows, where
/// the Job Object created at spawn owns the tree.
fn kill_group(pid: u32) {
    if cfg!(unix) {
        let _ = std::process::Command::new("kill")
            .args(["-KILL", "--", &format!("-{pid}")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

struct Handler {
    shared: Arc<Shared>,
    name: String,
    generation: u64,
}

impl Handler {
    fn refresh(&self, ctx: NotificationContext<RoleClient>, tools: bool) {
        let (sh, name, generation) = (self.shared.clone(), self.name.clone(), self.generation);
        // Not awaited inline: listing needs the service loop that is delivering this notification.
        tokio::spawn(async move {
            if tools {
                let _ = sh.refresh_tools(&name, generation, &ctx.peer).await; // keep old list on failure
            } else {
                sh.refresh_counts(&name, generation, &ctx.peer).await;
            }
            sh.emit();
        });
    }
}

impl ClientHandler for Handler {
    fn get_info(&self) -> ClientConfig {
        let o = &self.shared.opts;
        ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new(&o.client_name, &o.client_version),
        )
    }

    async fn on_tool_list_changed(&self, context: NotificationContext<RoleClient>) {
        self.refresh(context, true);
    }

    async fn on_resource_list_changed(&self, context: NotificationContext<RoleClient>) {
        self.refresh(context, false);
    }

    async fn on_prompt_list_changed(&self, context: NotificationContext<RoleClient>) {
        self.refresh(context, false);
    }
}

#[derive(Clone)]
pub struct McpManager {
    inner: Arc<Shared>,
}

impl McpManager {
    /// Registers servers; nothing is spawned or connected until [`start`](Self::start) or
    /// [`reconnect`](Self::reconnect).
    pub fn new(servers: Vec<McpServerConfig>, opts: McpOptions) -> Self {
        let entries: Vec<Entry> = servers
            .into_iter()
            .map(|cfg| Entry {
                state: if cfg.disabled {
                    ServerState::Disabled
                } else {
                    ServerState::Connecting
                },
                error: cfg.disabled.then(|| cfg.disabled_reason.clone()).flatten(),
                cfg,
                conn: None,
                generation: 0,
                kind: None,
                tools: vec![],
                resource_count: 0,
                prompt_count: 0,
                server_info: None,
                stderr: VecDeque::new(),
            })
            .collect();
        let (status_tx, _) = watch::channel(entries.iter().map(Entry::status).collect());
        Self {
            inner: Arc::new(Shared {
                opts,
                entries: Mutex::new(entries),
                names: Mutex::new(HashMap::new()),
                status_tx,
            }),
        }
    }

    fn names(&self) -> Vec<String> {
        lock(&self.inner.entries)
            .iter()
            .map(|e| e.cfg.name.clone())
            .collect()
    }

    /// Connects every enabled server in parallel; outcomes are in `status()`.
    pub async fn start(&self) {
        self.run_all(self.names()).await;
    }

    async fn run_all(&self, names: Vec<String>) {
        let tasks: Vec<_> = names
            .into_iter()
            .map(|n| {
                let sh = self.inner.clone();
                tokio::spawn(async move { sh.connect_one(&n).await })
            })
            .collect();
        for t in tasks {
            let _ = t.await;
        }
    }

    pub fn status(&self) -> Vec<ServerStatus> {
        lock(&self.inner.entries)
            .iter()
            .map(Entry::status)
            .collect()
    }

    /// Fires whenever any server's status changes (Node's `update` event).
    pub fn subscribe(&self) -> watch::Receiver<Vec<ServerStatus>> {
        self.inner.status_tx.subscribe()
    }

    /// Tools of connected servers. Also records the wire name -> (server, tool) reverse table.
    pub fn tools(&self) -> Vec<McpTool> {
        let mut out = Vec::new();
        let mut names = lock(&self.inner.names);
        for e in lock(&self.inner.entries)
            .iter()
            .filter(|e| e.state == ServerState::Connected)
        {
            for t in &e.tools {
                let name = tool_name_for(&e.cfg.name, &t.name);
                names.insert(name.clone(), (e.cfg.name.clone(), t.name.clone()));
                out.push(McpTool {
                    name,
                    server: e.cfg.name.clone(),
                    tool: t.name.clone(),
                    description: truncate(
                        &format!(
                            "[MCP {}] {}",
                            e.cfg.name,
                            t.description
                                .as_deref()
                                .filter(|d| !d.is_empty())
                                .unwrap_or(&t.name)
                        ),
                        MAX_DESCRIPTION_LEN,
                    ),
                    input_schema: t.input_schema.clone(),
                });
            }
        }
        out
    }

    /// Reverse of the wire name: the table filled by `tools()` (exact, survives truncation), else
    /// the `__` split.
    pub fn parse_tool_name(&self, name: &str) -> Option<(String, String)> {
        lock(&self.inner.names)
            .get(name)
            .cloned()
            .or_else(|| parse_tool_name_fallback(name))
    }

    /// `tools/call` by wire name.
    pub async fn call_wire(
        &self,
        wire_name: &str,
        input: Value,
        cancel: &CancellationToken,
    ) -> Result<McpToolResult, Aborted> {
        match self.parse_tool_name(wire_name) {
            Some((s, t)) => self.call_tool(&s, &t, input, cancel).await,
            None => Ok(McpToolResult::error(format!(
                "unknown MCP tool \"{wire_name}\""
            ))),
        }
    }

    /// `tools/call`. Server and transport failures come back as `is_error` results; only caller
    /// cancellation is `Err(Aborted)`. Dropping the request sends `notifications/cancelled`.
    pub async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        input: Value,
        cancel: &CancellationToken,
    ) -> Result<McpToolResult, Aborted> {
        let target = lock(&self.inner.entries)
            .iter()
            .find(|e| e.cfg.name == server && e.state == ServerState::Connected)
            .and_then(|e| {
                e.conn
                    .as_ref()
                    .map(|c| (c.peer.clone(), e.generation, e.cfg.clone()))
            });
        let Some((peer, generation, cfg)) = target else {
            return Ok(McpToolResult::error(format!(
                "MCP {server}: not connected."
            )));
        };
        if cancel.is_cancelled() {
            return Err(Aborted);
        }
        let args = match input {
            Value::Null => serde_json::Map::new(),
            Value::Object(o) => o,
            _ => {
                return Ok(McpToolResult::error(format!(
                    "MCP {server}/{tool}: arguments must be a JSON object"
                )));
            }
        };
        let mut params = CallToolRequestParams::new(tool.to_string());
        params.arguments = Some(args);
        let timeout = self.inner.opts.tool_timeout;
        let outcome = tokio::select! {
            _ = cancel.cancelled() => return Err(Aborted),
            r = tokio::time::timeout(timeout, peer.call_tool_once(params)) => r,
        };
        let msg = match outcome {
            Ok(Ok(CallToolResponse::Complete(r))) => {
                return Ok(serde_json::to_value(&r).map_or_else(
                    |e| McpToolResult::error(format!("MCP {server}/{tool}: bad result: {e}")),
                    |v| normalize_result(&v),
                ));
            }
            Ok(Ok(_)) => {
                "server requested multi-round input or a task, which is not supported".to_string()
            }
            Ok(Err(e)) => e.to_string(),
            Err(_) => format!("tool call timed out after {}ms", timeout.as_millis()),
        };
        let msg = self.inner.short_error(&cfg, &msg);
        if peer.is_transport_closed() {
            // Node leaves the entry "connected" after a crash; surface it instead.
            let e = format!("connection closed: {msg}");
            self.inner.with(server, generation, |en| {
                en.reset();
                en.state = ServerState::Failed;
                en.error = Some(e);
            });
            self.inner.emit();
        }
        Ok(McpToolResult::error(format!("MCP {server}/{tool}: {msg}")))
    }

    /// Closes and reconnects one server (a disabled server stays disabled).
    pub async fn reconnect(&self, name: &str) {
        self.inner.close_entry(name).await;
        self.inner.connect_one(name).await;
    }

    pub async fn reconnect_all(&self) {
        for n in self.names() {
            self.inner.close_entry(&n).await;
        }
        self.run_all(self.names()).await;
    }

    pub async fn set_enabled(&self, name: &str, enabled: bool) {
        {
            let mut g = lock(&self.inner.entries);
            let Some(e) = g.iter_mut().find(|e| e.cfg.name == name) else {
                return;
            };
            e.cfg.disabled = !enabled;
        }
        if enabled {
            self.inner.connect_one(name).await;
        } else {
            self.inner.close_entry(name).await;
            let mut g = lock(&self.inner.entries);
            if let Some(e) = g.iter_mut().find(|e| e.cfg.name == name) {
                e.state = ServerState::Disabled;
                e.error = e.cfg.disabled_reason.clone();
            }
            drop(g);
            self.inner.emit();
        }
    }

    /// Child pid of a connected stdio server.
    pub fn server_pid(&self, name: &str) -> Option<u32> {
        lock(&self.inner.entries)
            .iter()
            .find(|e| e.cfg.name == name)?
            .conn
            .as_ref()?
            .pid
    }

    /// Last (up to 50) stderr lines of a stdio server.
    pub fn stderr_tail(&self, name: &str) -> Vec<String> {
        lock(&self.inner.entries)
            .iter()
            .find(|e| e.cfg.name == name)
            .map(|e| e.stderr.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Closes every server. Stdio children get stdin EOF, 3 s to exit, then their whole process
    /// group (Job Object on Windows) is killed. Call before the runtime shuts down; dropping the
    /// manager only kills children asynchronously via rmcp's drop guard.
    pub async fn close(&self) {
        let tasks: Vec<_> = self
            .names()
            .into_iter()
            .map(|n| {
                let sh = self.inner.clone();
                tokio::spawn(async move { sh.close_entry(&n).await })
            })
            .collect();
        for t in tasks {
            let _ = t.await;
        }
        self.inner.emit();
    }
}
