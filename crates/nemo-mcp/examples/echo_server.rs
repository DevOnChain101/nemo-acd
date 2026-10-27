//! Test MCP server over stdio. Flags: `--list-changed` (adds tool `extra` after 300 ms and sends
#![allow(clippy::field_reassign_with_default)] // rmcp result structs are non_exhaustive
//! `tools/list_changed`), `--page-size N` (paginates `tools/list`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::{NotificationContext, RequestContext};
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::{Value, json};

// 1x1 transparent PNG
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

#[derive(Clone)]
struct Echo {
    list_changed: bool,
    page_size: usize,
    extra: Arc<AtomicBool>,
}

fn tool(name: &'static str, desc: &'static str, schema: Value) -> Tool {
    Tool::new(name, desc, Arc::new(schema.as_object().cloned().unwrap()))
}

impl Echo {
    fn tools(&self) -> Vec<Tool> {
        let obj = json!({"type":"object","properties":{}});
        let mut t = vec![
            tool(
                "echo",
                "Echoes text.",
                json!({"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}),
            ),
            tool(
                "add",
                "Adds a and b.",
                json!({"type":"object","properties":{"a":{"type":"number"},"b":{"type":"number"}}}),
            ),
            tool("fail", "Returns an error result.", obj.clone()),
            tool(
                "slow",
                "Sleeps ms milliseconds.",
                json!({"type":"object","properties":{"ms":{"type":"number"}}}),
            ),
            tool("image", "Returns text plus an image.", obj.clone()),
            tool(
                "env",
                "Returns the value of env var `name` or <unset>.",
                json!({"type":"object","properties":{"name":{"type":"string"}}}),
            ),
            tool("crash", "Exits the process mid-call.", obj.clone()),
            tool(
                "spawn_child",
                "Spawns a long-lived grandchild and returns its pid.",
                obj.clone(),
            ),
        ];
        if self.extra.load(Ordering::SeqCst) {
            t.push(tool("extra", "Added after startup.", obj));
        }
        t
    }
}

fn text(s: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(s)])
}

impl ServerHandler for Echo {
    fn get_info(&self) -> ServerConfig {
        let mut caps = ServerCapabilities::builder().enable_tools();
        if self.list_changed {
            caps = caps.enable_tool_list_changed();
        }
        ServerConfig::new(caps.build())
    }

    async fn on_initialized(&self, ctx: NotificationContext<RoleServer>) {
        eprintln!("echo-server ready");
        if self.list_changed {
            let extra = self.extra.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(300)).await;
                extra.store(true, Ordering::SeqCst);
                let _ = ctx.peer.notify_tool_list_changed().await;
            });
        }
    }

    async fn list_tools(
        &self,
        req: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let all = self.tools();
        let start = req
            .and_then(|r| r.cursor)
            .and_then(|c| c.parse::<usize>().ok())
            .unwrap_or(0);
        let size = if self.page_size == 0 {
            all.len()
        } else {
            self.page_size
        };
        let end = (start + size).min(all.len());
        let mut r = ListToolsResult::default();
        r.tools = all[start..end].to_vec();
        r.next_cursor = (end < all.len()).then(|| end.to_string());
        Ok(r)
    }

    async fn call_tool(
        &self,
        req: CallToolRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let a = req.arguments.unwrap_or_default();
        let n = |k: &str| a.get(k).and_then(Value::as_f64).unwrap_or(0.0);
        let r = match req.name.as_ref() {
            "echo" => text(a.get("text").and_then(Value::as_str).unwrap_or("")),
            "add" => text((n("a") + n("b")).to_string()),
            "fail" => CallToolResult::error(vec![ContentBlock::text("intentional failure")]),
            "slow" => {
                eprintln!("slow started");
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(n("ms") as u64)) => text("done"),
                    _ = ctx.ct.cancelled() => { eprintln!("slow cancelled"); text("cancelled") }
                }
            }
            "image" => CallToolResult::success(vec![
                ContentBlock::text("pic"),
                ContentBlock::image(PNG, "image/png"),
            ]),
            "env" => text(
                std::env::var(a.get("name").and_then(Value::as_str).unwrap_or(""))
                    .unwrap_or_else(|_| "<unset>".into()),
            ),
            "crash" => std::process::exit(3),
            "spawn_child" => {
                // Same process group as this server, so a group kill must reap it too.
                let cmd = if cfg!(windows) {
                    ("ping", vec!["-n", "300", "127.0.0.1"])
                } else {
                    ("sleep", vec!["300"])
                };
                let child = std::process::Command::new(cmd.0)
                    .args(cmd.1)
                    .spawn()
                    .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
                text(child.id().to_string())
            }
            "extra" => text("extra"),
            other => {
                return Err(ErrorData::invalid_params(
                    format!("unknown tool {other}"),
                    None,
                ));
            }
        };
        Ok(CallToolResponse::Complete(r))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let page_size = args
        .iter()
        .position(|a| a == "--page-size")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let server = Echo {
        list_changed: args.iter().any(|a| a == "--list-changed"),
        page_size,
        extra: Arc::new(AtomicBool::new(false)),
    };
    server
        .serve(rmcp::transport::stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}
