#![allow(clippy::field_reassign_with_default)] // rmcp result structs are non_exhaustive
mod common;
use std::sync::Arc;

use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::Response;
use common::*;
use nemo_config::Env;
use nemo_mcp::{McpManager, McpServerConfig, ServerState};
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use rmcp::{ErrorData, RoleServer, ServerHandler};
use serde_json::json;

#[derive(Clone)]
struct Mini;

impl ServerHandler for Mini {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let mut r = ListToolsResult::default();
        r.tools = vec![Tool::new(
            "ping",
            "pong",
            Arc::new(
                json!({"type":"object","properties":{}})
                    .as_object()
                    .cloned()
                    .unwrap(),
            ),
        )];
        Ok(r)
    }
    async fn call_tool(
        &self,
        _: CallToolRequestParams,
        _: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        Ok(CallToolResponse::Complete(CallToolResult::success(vec![
            ContentBlock::text("pong"),
        ])))
    }
}

async fn require_bearer(req: Request, next: Next) -> Result<Response, StatusCode> {
    let ok = req
        .headers()
        .get("authorization")
        .is_some_and(|v| v == "Bearer tok-123")
        && req
            .headers()
            .get("x-custom")
            .is_some_and(|v| v == "from-env-abc");
    if ok {
        Ok(next.run(req).await)
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

async fn serve() -> String {
    let svc = StreamableHttpService::new(
        || Ok(Mini),
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default(),
    );
    let app = axum::Router::new()
        .nest_service("/mcp", svc)
        .layer(middleware::from_fn(require_bearer));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/mcp", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    url
}

#[tokio::test]
async fn streamable_http_with_headers() {
    let url = serve().await;
    let mut cfg = McpServerConfig::http("web", url.clone());
    cfg.headers
        .insert("Authorization".into(), "Bearer ${TOK}".into());
    cfg.headers
        .insert("X-Custom".into(), "from-env-${SUFFIX}".into());
    cfg.scope = Some("user".into());
    let env = Env::from_pairs([("TOK", "tok-123"), ("SUFFIX", "abc")]);
    let mut o = opts(env);
    o.env_passthrough = vec!["TOK".into(), "SUFFIX".into()];
    let m = McpManager::new(vec![cfg], o);
    m.start().await;
    let st = &m.status()[0];
    assert_eq!(
        (st.state, st.transport, st.tool_count),
        (ServerState::Connected, "http", 1),
        "{st:?}"
    );
    assert_eq!(call(&m, "web", "ping", json!({})).await.text(), "pong");
    m.close().await;

    // wrong token: Failed, and the token never shows in the error
    let mut bad = McpServerConfig::http("web", url);
    bad.headers
        .insert("Authorization".into(), "Bearer wrong-token-77".into());
    let m = McpManager::new(vec![bad], opts(Env::default()));
    m.start().await;
    let st = &m.status()[0];
    assert_eq!(st.state, ServerState::Failed);
    assert!(!st.error.as_deref().unwrap().contains("wrong-token-77"));
    m.close().await;
}

#[tokio::test]
async fn non_bearer_authorization_is_rejected_clearly() {
    let mut cfg = McpServerConfig::http("web", "http://127.0.0.1:1/mcp");
    cfg.headers
        .insert("Authorization".into(), "Basic abc".into());
    let m = McpManager::new(vec![cfg], opts(Env::default()));
    m.start().await;
    assert!(m.status()[0].error.as_deref().unwrap().contains("Bearer"));
}
