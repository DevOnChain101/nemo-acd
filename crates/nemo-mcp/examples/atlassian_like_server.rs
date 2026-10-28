#![allow(clippy::field_reassign_with_default)] // rmcp result structs are non_exhaustive
//! Test MCP server (stdio) that mimics Confluence/Jira write semantics.
//!
//! v2-style tools too (mcp.atlassian.com/v2/mcp): `getConfluenceContent(pageId)`,
//! `updateConfluenceContent(pageId, version?, body | edits[{find,replace}])` (full-body replacement
//! or granular edits), `listConfluenceContentVersions`, `editJiraIssue`, and the generic
//! `executeRead` / `executeWrite` / `executeDestructive` that take `{tool, arguments}`.
//!
//! Tools: `getPage`, `updatePage` (optimistic concurrency: `version.number` must be current+1,
//! the body REPLACES the stored one), `getIssue`, `editIssue` (given fields replace the stored
//! ones), `searchIssues`. Reads are annotated `readOnlyHint`, writes `destructiveHint`; results
//! carry `structuredContent` and the same JSON as text. Flags: `--no-annotations` (like servers
//! that send none), `--big-body` (page 100 body is 60 000 chars: the `version` field sits behind
//! it in the JSON), `--instructions` (adds server `instructions`).
//!
//! Identity: every write records `author` (and returns it) and `whoami` returns the caller. Over
//! stdio the caller is `JIRA_USERNAME` when `JIRA_API_TOKEN` is also set, else `anonymous` (what a
//! real Atlassian server shows for an unauthenticated write). Over HTTP the embedding test gives
//! `(header, exact value, principal)` rules; no matching header means `anonymous`.
//! `triggerListChanged` adds the tool `extraTool` and sends `tools/list_changed`.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool, ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::{Value, json};

#[derive(Clone)]
pub struct Atlassian {
    pub annotations: bool,
    pub instructions: bool,
    pub pages: Arc<Mutex<BTreeMap<String, Value>>>,
    pub issues: Arc<Mutex<BTreeMap<String, Value>>>,
    /// Caller when there is no HTTP request (stdio).
    pub fixed: String,
    /// `(lowercase header name, exact value, principal)`; consulted for HTTP requests.
    pub rules: Vec<(String, String, String)>,
    pub extra: Arc<AtomicBool>,
}

impl Atlassian {
    pub fn seeded(annotations: bool, instructions: bool, big_body: bool, fixed: String) -> Self {
        let body = if big_body {
            format!("{BODY}{}", "filler line of the page\n".repeat(2500))
        } else {
            BODY.to_string()
        };
        let page = json!({"id":"100","title":"Runbook","body":body,"version":{"number":3}});
        let issue = json!({"id":"10001","key":"PROJ-1","fields":{
            "summary":"Login fails","description":"Steps: 1. open login. 2. submit.\nExpected: ok.","labels":["bug"]}});
        let other = json!({"id":"10002","key":"PROJ-2","fields":{"summary":"Slow search","description":"p95 2s","labels":[]}});
        Atlassian {
            annotations,
            instructions,
            pages: Arc::new(Mutex::new([("100".to_string(), page)].into())),
            issues: Arc::new(Mutex::new(
                [("PROJ-1".to_string(), issue), ("PROJ-2".to_string(), other)].into(),
            )),
            fixed,
            rules: vec![],
            extra: Arc::new(AtomicBool::new(false)),
        }
    }

    fn who(&self, ctx: &RequestContext<RoleServer>) -> String {
        let Some(parts) = ctx.extensions.get::<http::request::Parts>() else {
            return self.fixed.clone();
        };
        self.rules
            .iter()
            .find(|(h, v, _)| {
                parts
                    .headers
                    .get(h.as_str())
                    .is_some_and(|x| x == v.as_str())
            })
            .map_or_else(|| "anonymous".to_string(), |(_, _, p)| p.clone())
    }
}

fn tool(name: &'static str, desc: &'static str, schema: Value, a: Option<ToolAnnotations>) -> Tool {
    let t = Tool::new(name, desc, Arc::new(schema.as_object().cloned().unwrap()));
    match a {
        Some(a) => t.with_annotations(a),
        None => t,
    }
}

fn reply(v: Value, is_error: bool) -> CallToolResult {
    let mut r = if is_error {
        CallToolResult::error(vec![ContentBlock::text(v.to_string())])
    } else {
        CallToolResult::success(vec![ContentBlock::text(v.to_string())])
    };
    if !is_error {
        r.structured_content = Some(v);
    }
    r
}

fn err(msg: String) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(msg)])
}

impl Atlassian {
    fn tools(&self) -> Vec<Tool> {
        let ro = || {
            self.annotations
                .then(|| ToolAnnotations::new().read_only(true))
        };
        let rw = |idempotent: bool| {
            self.annotations.then(|| {
                let mut a = ToolAnnotations::new().read_only(false).destructive(true);
                a.idempotent_hint = Some(idempotent);
                a
            })
        };
        let obj = || json!({"type":"object","properties":{"tool":{"type":"string"},"arguments":{"type":"object"}},"required":["tool","arguments"]});
        let v2 = vec![
            tool(
                "getConfluenceContent",
                "Get a Confluence page by pageId.",
                json!({"type":"object","properties":{"pageId":{"type":"string"}},"required":["pageId"]}),
                ro(),
            ),
            tool(
                "updateConfluenceContent",
                "Update a page via full-body replacement (body) or granular edits (edits).",
                json!({"type":"object","properties":{"pageId":{"type":"string"},"title":{"type":"string"},
                    "body":{"type":"string"},"edits":{"type":"array"},"version":{"type":"object"}},"required":["pageId"]}),
                rw(false),
            ),
            tool(
                "listConfluenceContentVersions",
                "List the versions of a page.",
                json!({"type":"object","properties":{"pageId":{"type":"string"}},"required":["pageId"]}),
                ro(),
            ),
            tool(
                "editJiraIssue",
                "Edit a Jira issue.",
                json!({"type":"object","properties":{"issueIdOrKey":{"type":"string"},"fields":{"type":"object"}},"required":["issueIdOrKey","fields"]}),
                rw(true),
            ),
            tool("executeRead", "Run a read tool by name.", obj(), ro()),
            tool("executeWrite", "Run a write tool by name.", obj(), rw(true)),
            tool(
                "executeDestructive",
                "Run a destructive tool by name.",
                obj(),
                self.annotations
                    .then(|| ToolAnnotations::new().read_only(false).destructive(true)),
            ),
        ];
        let mut v1 = vec![
            tool(
                "getPage",
                "Get a Confluence page: id, title, body and version.number.",
                json!({"type":"object","properties":{"id":{"type":"string"}},"required":["id"]}),
                ro(),
            ),
            tool(
                "updatePage",
                "Update a Confluence page. The body REPLACES the stored body; version.number must be the current version + 1.",
                json!({"type":"object","properties":{
                    "id":{"type":"string"},"title":{"type":"string"},"body":{"type":"string"},
                    "version":{"type":"object","properties":{"number":{"type":"number"}},"required":["number"]}
                },"required":["id","title","body","version"]}),
                rw(false),
            ),
            tool(
                "getIssue",
                "Get a Jira issue.",
                json!({"type":"object","properties":{"issueIdOrKey":{"type":"string"}},"required":["issueIdOrKey"]}),
                ro(),
            ),
            tool(
                "editIssue",
                "Edit a Jira issue; each given field replaces the stored one.",
                json!({"type":"object","properties":{"issueIdOrKey":{"type":"string"},
                    "fields":{"type":"object"}},"required":["issueIdOrKey","fields"]}),
                rw(true),
            ),
            tool(
                "searchIssues",
                "Search Jira issues (substring match on summary).",
                json!({"type":"object","properties":{"jql":{"type":"string"}},"required":["jql"]}),
                ro(),
            ),
        ];
        v1.extend(v2);
        v1.push(tool(
            "whoami",
            "The authenticated principal of this connection.",
            json!({"type":"object","properties":{}}),
            ro(),
        ));
        v1.push(tool(
            "triggerListChanged",
            "Test helper: adds extraTool and sends tools/list_changed.",
            json!({"type":"object","properties":{}}),
            ro(),
        ));
        if self.extra.load(Ordering::SeqCst) {
            v1.push(tool(
                "extraTool",
                "Appears after triggerListChanged.",
                json!({"type":"object","properties":{}}),
                ro(),
            ));
        }
        v1
    }

    fn call(&self, name: &str, a: &serde_json::Map<String, Value>, who: &str) -> CallToolResult {
        if name.starts_with("execute") {
            let inner = a
                .get("arguments")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            return self.call(
                a.get("tool").and_then(Value::as_str).unwrap_or(""),
                &inner,
                who,
            );
        }
        let s = |k: &str| {
            a.get(k)
                .or_else(|| (k == "id").then(|| a.get("pageId")).flatten())
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        match name {
            "getConfluenceContent" => self.call(
                "getPage",
                &[("id".to_string(), json!(s("pageId")))]
                    .into_iter()
                    .collect(),
                who,
            ),
            "listConfluenceContentVersions" => match self.pages.lock().unwrap().get(&s("pageId")) {
                Some(p) => reply(
                    json!({"id": s("pageId"), "versions": (1..=p["version"]["number"].as_i64().unwrap_or(1)).map(|n| json!({"number": n})).collect::<Vec<_>>()}),
                    false,
                ),
                None => err("404".into()),
            },
            "editJiraIssue" => self.call("editIssue", a, who),
            "updateConfluenceContent" => {
                let mut pages = self.pages.lock().unwrap();
                let Some(p) = pages.get_mut(&s("pageId")) else {
                    return err("404".into());
                };
                let cur = p["version"]["number"].as_i64().unwrap_or(0);
                if let Some(want) = a.get("version").and_then(|v| v["number"].as_i64())
                    && want != cur + 1
                {
                    return err(format!("409 Conflict: version.number must be {}", cur + 1));
                }
                if let Some(b) = a.get("body").and_then(Value::as_str) {
                    p["body"] = json!(b);
                }
                for e in a
                    .get("edits")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let body = p["body"].as_str().unwrap_or("").replacen(
                        e["find"].as_str().unwrap_or("\u{0}"),
                        e["replace"].as_str().unwrap_or(""),
                        1,
                    );
                    p["body"] = json!(body);
                }
                p["version"] = json!({"number": cur + 1});
                p["author"] = json!(who);
                reply(
                    json!({"id": s("pageId"), "version": {"number": cur + 1}, "author": who}),
                    false,
                )
            }
            "getPage" => match self.pages.lock().unwrap().get(&s("id")) {
                Some(p) => reply(p.clone(), false),
                None => err(format!("404 page {} not found", s("id"))),
            },
            "updatePage" => {
                let mut pages = self.pages.lock().unwrap();
                let Some(p) = pages.get_mut(&s("id")) else {
                    return err(format!("404 page {} not found", s("id")));
                };
                let cur = p["version"]["number"].as_i64().unwrap_or(0);
                let want = a["version"]["number"].as_i64().unwrap_or(-1);
                if want != cur + 1 {
                    return err(format!(
                        "409 Conflict: version.number must be {} (current is {cur}), got {want}",
                        cur + 1
                    ));
                }
                // Replaces the whole body: anything the caller did not send is gone.
                p["title"] = json!(s("title"));
                p["body"] = json!(s("body"));
                p["version"] = json!({"number": want});
                p["author"] = json!(who);
                reply(
                    json!({"id": s("id"), "version": {"number": want}, "author": who}),
                    false,
                )
            }
            "getIssue" => match self.issues.lock().unwrap().get(&s("issueIdOrKey")) {
                Some(i) => reply(i.clone(), false),
                None => err(format!("404 issue {} not found", s("issueIdOrKey"))),
            },
            "editIssue" => {
                let mut issues = self.issues.lock().unwrap();
                let Some(i) = issues.get_mut(&s("issueIdOrKey")) else {
                    return err(format!("404 issue {} not found", s("issueIdOrKey")));
                };
                for (k, v) in a["fields"].as_object().cloned().unwrap_or_default() {
                    i["fields"][k] = v;
                }
                i["lastModifiedBy"] = json!(who);
                reply(json!({"key": i["key"], "ok": true, "author": who}), false)
            }
            "searchIssues" => {
                let q = s("jql").to_lowercase();
                let hits: Vec<Value> = self
                    .issues
                    .lock()
                    .unwrap()
                    .values()
                    .filter(|i| {
                        q.is_empty()
                            || i["fields"]["summary"]
                                .as_str()
                                .is_some_and(|t| t.to_lowercase().contains(&q))
                    })
                    .map(|i| json!({"key": i["key"], "summary": i["fields"]["summary"]}))
                    .collect();
                reply(json!({"issues": hits}), false)
            }
            "whoami" => reply(
                json!({"principal": who, "authenticated": who != "anonymous"}),
                false,
            ),
            other => err(format!("unknown tool {other}")),
        }
    }
}

impl ServerHandler for Atlassian {
    fn get_info(&self) -> ServerConfig {
        let c = ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_tool_list_changed()
                .build(),
        );
        if self.instructions {
            c.with_instructions("Always fetch a page before updating it.")
        } else {
            c
        }
    }

    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let mut r = ListToolsResult::default();
        r.tools = self.tools();
        Ok(r)
    }

    async fn call_tool(
        &self,
        req: CallToolRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let a = req.arguments.unwrap_or_default();
        if req.name == "triggerListChanged" {
            self.extra.store(true, Ordering::SeqCst);
            let peer = ctx.peer.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                let _ = peer.notify_tool_list_changed().await;
            });
            return Ok(CallToolResponse::Complete(reply(
                json!({"ok": true}),
                false,
            )));
        }
        let who = self.who(&ctx);
        Ok(CallToolResponse::Complete(self.call(&req.name, &a, &who)))
    }
}

const BODY: &str = "# Runbook\n\n## Rollback\nStep 1: stop the deploy.\nStep 2: restore the snapshot.\n\n## Contacts\nOn call: alice@example.com\nEscalation: bob@example.com\n\n## History\n2024-01-05 first version\n2024-02-11 added contacts\n";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let flag = |f: &str| std::env::args().any(|a| a == f);
    let var = |k: &str| std::env::var(k).unwrap_or_default();
    let who = if var("JIRA_USERNAME").is_empty() || var("JIRA_API_TOKEN").is_empty() {
        "anonymous".to_string()
    } else {
        var("JIRA_USERNAME")
    };
    let server = Atlassian::seeded(
        !flag("--no-annotations"),
        flag("--instructions"),
        flag("--big-body"),
        who,
    );
    server
        .serve(rmcp::transport::stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}
