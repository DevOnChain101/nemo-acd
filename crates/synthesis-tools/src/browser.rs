//! browser_* tools on top of `synthesis_browser` (tools.md, doc §5.9). One lazily launched
//! `BrowserRuntime` per session lives in `SessionState`; it dies with the session. Every string
//! that came from a page is wrapped as `UntrustedText` so the harness can label it as data.

use std::error::Error;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use serde_json::{Value, json};
use synthesis_browser::{
    BrowserError, BrowserOptions, BrowserRuntime, PageInfo, UntrustedText, UrlPolicy, WaitFor,
    WaitUntil,
};
use synthesis_config::Env;
use synthesis_core::ToolContext;

use crate::native::{Shared, lock};
use crate::util::{int_field, str_field};
use crate::{Block, BoxFuture, Content, ImageSource, ToolOutput};

/// Browser settings of the host (set with `Registry::with_browser`).
#[derive(Debug, Clone, Default)]
pub struct BrowserConfig {
    pub policy: UrlPolicy,
    /// Only for containers where Chrome's own sandbox cannot start.
    pub no_sandbox: bool,
    /// The provider accepts image blocks in tool results: `browser_screenshot` returns the PNG.
    pub supports_images: bool,
}

type Res = Result<ToolOutput, Box<dyn Error + Send + Sync>>;

/// Input of a tool call with every secret removed, safe for events and logs. Today that is the
/// `value` of `browser_fill` with `sensitive: true`.
pub fn redact_input(tool: &str, input: &Value) -> Value {
    let mut v = input.clone();
    if tool == "browser_fill"
        && v["sensitive"] == true
        && let Some(o) = v.as_object_mut()
    {
        o.insert("value".into(), "[REDACTED]".into());
    }
    v
}

async fn runtime(
    sh: &Shared,
    ctx: &ToolContext,
    tool: &str,
) -> Result<Arc<BrowserRuntime>, ToolOutput> {
    let cfg = lock(&sh.browser_cfg).clone();
    sh.session(ctx)
        .browser
        .get_or_try_init(|| async {
            let mut o = BrowserOptions::new(ctx.workspace.artifact_root.join("browser"));
            o.policy = cfg.policy;
            o.no_sandbox = cfg.no_sandbox;
            let env = Env::from_pairs(ctx.env.iter().map(|(k, v)| (k.clone(), v.clone())));
            BrowserRuntime::launch(o, &env).await.map(Arc::new)
        })
        .await
        .cloned()
        .map_err(|e| {
            ToolOutput::error(format!("{tool}: {e}"))
                .with_meta(json!({"error": "browser_unavailable"}))
        })
}

fn untrusted(s: String) -> String {
    UntrustedText::new(s).to_string()
}

fn page_text(p: &PageInfo) -> String {
    format!("url: {}\ntitle: {}", p.url.expose(), p.title.expose())
}

macro_rules! browser_tool {
    ($name:ident, $wire:literal, $body:ident) => {
        pub(crate) fn $name<'a>(
            sh: &'a Shared,
            input: &'a Value,
            ctx: &'a ToolContext,
        ) -> BoxFuture<'a, ToolOutput> {
            Box::pin(async move {
                let rt = match runtime(sh, ctx, $wire).await {
                    Ok(rt) => rt,
                    Err(e) => return e,
                };
                let r = tokio::select! {
                    r = $body(&rt, input, sh) => r,
                    () = ctx.cancel.cancelled() => Err("aborted.".into()),
                };
                r.unwrap_or_else(|e| {
                    let kind = match e.downcast_ref::<BrowserError>() {
                        Some(BrowserError::Policy(_)) => "url_policy",
                        Some(BrowserError::StaleRef(_)) => "stale_ref",
                        Some(BrowserError::Timeout(_)) => "timeout",
                        _ => "browser_error",
                    };
                    ToolOutput::error(format!("{}: {e}", $wire)).with_meta(json!({"error": kind}))
                })
            })
        }
    };
}
browser_tool!(navigate, "browser_navigate", nav);
browser_tool!(snapshot, "browser_snapshot", snap);
browser_tool!(click, "browser_click", click_body);
browser_tool!(fill, "browser_fill", fill_body);
browser_tool!(press_key, "browser_press_key", key);
browser_tool!(wait_for, "browser_wait_for", wait);
browser_tool!(screenshot, "browser_screenshot", shot);
browser_tool!(console, "browser_console", console_body);
browser_tool!(network, "browser_network", network_body);
browser_tool!(tabs, "browser_tabs", tabs_body);
browser_tool!(close, "browser_close", close_body);

async fn nav(rt: &BrowserRuntime, input: &Value, _: &Shared) -> Res {
    let url = str_field(input, "url").ok_or("\"url\" is required.")?;
    let until = match input["wait_until"].as_str() {
        Some("domcontentloaded") => WaitUntil::DomContentLoaded,
        Some("networkidle") => WaitUntil::NetworkIdle,
        _ => WaitUntil::Load,
    };
    let go = rt.navigate(url, until);
    let page = match int_field(input, "timeout_ms") {
        Some(t) => tokio::time::timeout(Duration::from_millis(t.clamp(1, 600_000) as u64), go)
            .await
            .map_err(|_| BrowserError::Timeout(format!("navigation to {url}")))??,
        None => go.await?,
    };
    Ok(ToolOutput::text(format!(
        "Navigated.\n{}",
        untrusted(page_text(&page))
    )))
}

async fn snap(rt: &BrowserRuntime, input: &Value, _: &Shared) -> Res {
    if input.get("ref").is_some() {
        return Err("subtree snapshots (ref) are not supported yet.".into());
    }
    let max = int_field(input, "max_nodes").unwrap_or(200).clamp(1, 2000) as usize;
    let s = rt.snapshot(max).await?;
    let more = if s.truncated {
        "\n[truncated: pass a larger max_nodes]"
    } else {
        ""
    };
    Ok(ToolOutput::text(format!(
        "{}{more}",
        untrusted(format!("{}\n\n{}", page_text(&s.page), s.tree.expose()))
    )))
}

/// The single ref whose line in the snapshot is `- <role> "<name>"`.
async fn ref_by_role(rt: &BrowserRuntime, role: &str, name: &str) -> Result<String, String> {
    let tree = rt.snapshot(2000).await.map_err(|e| e.to_string())?;
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let head = format!("- {role} {}", json!(name));
    let refs: Vec<&str> = tree
        .tree
        .expose()
        .lines()
        .filter(|l| l.trim_start().starts_with(&head))
        .filter_map(|l| l.split("[ref=").nth(1)?.split(']').next())
        .collect();
    match refs[..] {
        [r] => Ok(r.to_string()),
        [] => Err(format!("no element with role {role:?} and name {name:?}.")),
        _ => Err(format!(
            "role {role:?} and name {name:?} match {} elements ({}); use a ref.",
            refs.len(),
            refs.join(", ")
        )),
    }
}

async fn click_body(rt: &BrowserRuntime, input: &Value, _: &Shared) -> Res {
    let r = match (
        str_field(input, "ref"),
        str_field(input, "role"),
        str_field(input, "name"),
    ) {
        (Some(r), ..) => r.to_string(),
        (None, Some(role), Some(name)) => ref_by_role(rt, role, name).await?,
        _ if input.get("selector").is_some() => {
            return Err("selectors are not supported yet; use ref or role+name.".into());
        }
        _ => return Err("give ref, or role and name.".into()),
    };
    rt.click(&r).await?;
    Ok(ToolOutput::text(format!("Clicked {r}.")))
}

async fn fill_body(rt: &BrowserRuntime, input: &Value, _: &Shared) -> Res {
    let Some(r) = str_field(input, "ref") else {
        return Err(if input.get("selector").is_some() {
            "selectors are not supported yet; use ref."
        } else {
            "\"ref\" is required."
        }
        .into());
    };
    let value = input["value"].as_str().unwrap_or("");
    rt.fill(r, value).await?;
    // The text is never echoed: a sensitive value must not reach the transcript or events.
    Ok(ToolOutput::text(format!(
        "Filled {r} ({} characters).",
        value.chars().count()
    ))
    .with_meta(json!({"ref": r, "sensitive": input["sensitive"] == true})))
}

async fn key(rt: &BrowserRuntime, input: &Value, _: &Shared) -> Res {
    let k = str_field(input, "key").ok_or("\"key\" is required.")?;
    rt.press_key(k).await?;
    Ok(ToolOutput::text(format!("Pressed {k}.")))
}

async fn wait(rt: &BrowserRuntime, input: &Value, _: &Shared) -> Res {
    let t = int_field(input, "timeout_ms")
        .unwrap_or(10_000)
        .clamp(1, 120_000) as u64;
    let what = match (
        str_field(input, "text"),
        str_field(input, "selector"),
        input["network_idle"] == true,
        int_field(input, "sleep_ms"),
    ) {
        (Some(s), None, false, None) => WaitFor::Text(s.into()),
        (None, Some(s), false, None) => WaitFor::Selector(s.into()),
        (None, None, true, None) => WaitFor::NetworkIdle,
        (None, None, false, Some(ms)) => WaitFor::Sleep(Duration::from_millis(ms.max(0) as u64)),
        _ => return Err("give exactly one of text, selector, network_idle, sleep_ms.".into()),
    };
    rt.wait_for(what, Duration::from_millis(t)).await?;
    Ok(ToolOutput::text("Done waiting."))
}

async fn shot(rt: &BrowserRuntime, input: &Value, sh: &Shared) -> Res {
    if input.get("ref").is_some() {
        return Err("element screenshots (ref) are not supported yet.".into());
    }
    let s = rt.screenshot(input["full_page"] == true).await?;
    let path = s.path.display().to_string();
    let meta = json!({"artifact_path": path});
    if lock(&sh.browser_cfg).supports_images {
        return Ok(ToolOutput {
            content: Content::Blocks(vec![
                Block::Image {
                    source: ImageSource::Base64 {
                        media_type: "image/png".into(),
                        data: base64::engine::general_purpose::STANDARD.encode(&s.png),
                    },
                },
                Block::Text {
                    text: format!("Screenshot saved as artifact {path}"),
                },
            ]),
            is_error: false,
            soft: false,
            meta,
        });
    }
    let t = rt.snapshot(80).await?;
    Ok(ToolOutput::text(format!(
        "Screenshot saved as artifact {path} (images are not supported by this model; page summary follows).\n{}",
        untrusted(format!("{}\n\n{}", page_text(&t.page), t.tree.expose()))
    ))
    .with_meta(meta))
}

fn cursor(input: &Value) -> Result<usize, Box<dyn Error + Send + Sync>> {
    str_field(input, "cursor")
        .map_or(Ok(0), |c| c.parse())
        .map_err(|_| "invalid cursor.".into())
}

async fn console_body(rt: &BrowserRuntime, input: &Value, _: &Shared) -> Res {
    let from = cursor(input)?;
    let (msgs, next) = rt.console_messages(from, str_field(input, "level"));
    let mut lines: Vec<String> = msgs
        .iter()
        .map(|m| format!("[{}] {}", m.level, m.text.expose()))
        .collect();
    if from == 0 {
        lines.extend(rt.exceptions().iter().map(|e| {
            format!(
                "[exception] {} ({}:{})",
                e.text.expose(),
                e.url.expose(),
                e.line.unwrap_or(0)
            )
        }));
    }
    let body = if lines.is_empty() {
        "(no console messages)".to_string()
    } else {
        untrusted(lines.join("\n"))
    };
    Ok(ToolOutput::text(format!("{body}\n[cursor {next}]"))
        .with_meta(json!({"cursor": next.to_string()})))
}

async fn network_body(rt: &BrowserRuntime, input: &Value, _: &Shared) -> Res {
    let (reqs, next) = rt.network_requests(cursor(input)?, input["failed_only"] == true);
    let lines: Vec<String> = reqs
        .iter()
        .map(|r| {
            format!(
                "{} {} -> {} {} {}ms{}",
                r.method,
                r.url.expose(),
                r.status.map_or("-".into(), |s| s.to_string()),
                r.resource_type,
                r.duration_ms.unwrap_or(0),
                r.error.as_ref().map_or(String::new(), |e| format!(" {e}")),
            )
        })
        .collect();
    let body = if lines.is_empty() {
        "(no requests)".to_string()
    } else {
        untrusted(lines.join("\n"))
    };
    Ok(ToolOutput::text(format!("{body}\n[cursor {next}]"))
        .with_meta(json!({"cursor": next.to_string()})))
}

async fn tabs_body(rt: &BrowserRuntime, input: &Value, _: &Shared) -> Res {
    if input["action"] == "select" {
        let i = int_field(input, "index").ok_or("select needs an index.")?;
        rt.select_tab(usize::try_from(i).map_err(|_| "invalid index.")?)
            .await?;
        return Ok(ToolOutput::text(format!("Selected tab {i}.")));
    }
    let lines: Vec<String> = rt
        .tabs()
        .await?
        .iter()
        .map(|t| {
            format!(
                "{}[{}] {} | {}",
                if t.active { "*" } else { " " },
                t.index,
                t.page.url.expose(),
                t.page.title.expose()
            )
        })
        .collect();
    Ok(ToolOutput::text(untrusted(lines.join("\n"))))
}

async fn close_body(rt: &BrowserRuntime, input: &Value, _: &Shared) -> Res {
    let i = match int_field(input, "index") {
        Some(i) => usize::try_from(i).map_err(|_| "invalid index.")?,
        None => rt
            .tabs()
            .await?
            .iter()
            .find(|t| t.active)
            .map_or(0, |t| t.index),
    };
    rt.close_tab(i).await?;
    Ok(ToolOutput::text(format!("Closed tab {i}.")))
}
