//! Console, exception and network capture, fed from CDP events. Bodies are never fetched.

use crate::cdp::CdpEvent;
use crate::policy::ip_is_private;
use crate::redact::{redact_text, redact_url};
use crate::untrusted::UntrustedText;
use serde_json::Value;
use std::collections::HashMap;
use std::time::Instant;

/// Per-kind cap. shortcut: later entries are dropped, upgrade to a ring buffer with cursors.
const MAX_ENTRIES: usize = 2000;

#[derive(Clone, Debug)]
pub struct ConsoleMessage {
    /// error | warning | info | debug
    pub level: String,
    pub text: UntrustedText,
}

#[derive(Clone, Debug)]
pub struct JsException {
    pub text: UntrustedText,
    pub url: UntrustedText,
    pub line: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct NetworkRequest {
    pub method: String,
    /// Userinfo and secret-looking query values are redacted.
    pub url: UntrustedText,
    pub resource_type: String,
    /// Final status of this hop (3xx for a redirect hop).
    pub status: Option<u16>,
    /// Chrome's error text, e.g. `net::ERR_CONNECTION_REFUSED`.
    pub error: Option<String>,
    pub duration_ms: Option<u64>,
    started: f64,
    counted: bool,
}

impl NetworkRequest {
    pub fn is_failure(&self) -> bool {
        self.error.is_some() || self.status.is_some_and(|s| s >= 400)
    }
}

pub(crate) struct Logs {
    pub console: Vec<ConsoleMessage>,
    pub exceptions: Vec<JsException>,
    pub network: Vec<NetworkRequest>,
    index: HashMap<String, usize>,
    pub in_flight: usize,
    /// Responses served from a private address are recorded as failures, never as a status.
    pub deny_private: bool,
    pub last_activity: Instant,
}

impl Default for Logs {
    fn default() -> Self {
        Self {
            console: vec![],
            exceptions: vec![],
            network: vec![],
            index: HashMap::new(),
            in_flight: 0,
            deny_private: false,
            last_activity: Instant::now(),
        }
    }
}

fn level(raw: &str) -> &'static str {
    match raw {
        "error" | "assert" => "error",
        "warning" | "warn" => "warning",
        "debug" | "trace" | "verbose" => "debug",
        _ => "info",
    }
}

fn arg_text(a: &Value) -> String {
    match &a["value"] {
        Value::String(s) => s.clone(),
        Value::Null => a["description"]
            .as_str()
            .or(a["type"].as_str())
            .unwrap_or("")
            .to_owned(),
        v => v.to_string(),
    }
}

fn ms_between(start: f64, end: f64) -> u64 {
    ((end - start).max(0.0) * 1000.0).round() as u64
}

impl Logs {
    pub fn handle(&mut self, ev: &CdpEvent) {
        let p = &ev.params;
        match ev.method.as_str() {
            "Runtime.consoleAPICalled" => {
                let text = p["args"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(arg_text)
                    .collect::<Vec<_>>()
                    .join(" ");
                self.push_console(level(p["type"].as_str().unwrap_or("log")), &text);
            }
            "Log.entryAdded" => {
                let e = &p["entry"];
                let text = format!(
                    "{} {}",
                    e["text"].as_str().unwrap_or(""),
                    e["url"].as_str().unwrap_or("")
                );
                self.push_console(level(e["level"].as_str().unwrap_or("info")), text.trim());
            }
            "Runtime.exceptionThrown" if self.exceptions.len() < MAX_ENTRIES => {
                let d = &p["exceptionDetails"];
                let text = d["exception"]["description"]
                    .as_str()
                    .or(d["text"].as_str())
                    .unwrap_or("");
                self.exceptions.push(JsException {
                    text: UntrustedText::new(redact_text(text)),
                    url: UntrustedText::new(redact_url(d["url"].as_str().unwrap_or(""))),
                    line: d["lineNumber"].as_i64(),
                });
            }
            "Network.requestWillBeSent" => self.request_sent(p),
            "Network.responseReceived" => {
                let private = self.deny_private
                    && p["response"]["remoteIPAddress"].as_str().is_some_and(|ip| {
                        ip.trim_matches(['[', ']']).parse().is_ok_and(ip_is_private)
                    });
                if let Some(r) = self.record(p) {
                    if private {
                        r.error = Some("blocked by URL policy: private remote address".into());
                    } else {
                        r.status = p["response"]["status"].as_u64().map(|s| s as u16);
                    }
                }
            }
            "Network.loadingFinished" => self.finish(p, None),
            "Network.loadingFailed" => {
                let err = p["errorText"].as_str().unwrap_or("failed").to_owned();
                self.finish(p, Some(err));
            }
            _ => return,
        }
        if ev.method.starts_with("Network.") {
            self.last_activity = Instant::now();
        }
    }

    fn push_console(&mut self, level: &str, text: &str) {
        if self.console.len() < MAX_ENTRIES {
            self.console.push(ConsoleMessage {
                level: level.to_owned(),
                text: UntrustedText::new(redact_text(text)),
            });
        }
    }

    fn record(&mut self, p: &Value) -> Option<&mut NetworkRequest> {
        let i = *self.index.get(p["requestId"].as_str()?)?;
        self.network.get_mut(i)
    }

    fn request_sent(&mut self, p: &Value) {
        let Some(id) = p["requestId"].as_str() else {
            return;
        };
        let ts = p["timestamp"].as_f64().unwrap_or(0.0);
        let rtype = p["type"].as_str().unwrap_or("Other");
        // A redirect re-uses the request id: close the previous hop, keep one in-flight slot.
        let redirected = p.get("redirectResponse").is_some_and(|r| !r.is_null());
        if redirected && let Some(r) = self.record(p) {
            r.status = p["redirectResponse"]["status"].as_u64().map(|s| s as u16);
            r.duration_ms = Some(ms_between(r.started, ts));
        }
        let counted = if redirected {
            self.record(p).is_some_and(|r| r.counted)
        } else {
            // Streams stay open for good and would block "network idle".
            !matches!(rtype, "EventSource" | "Ping" | "Prefetch")
        };
        if counted && !redirected {
            self.in_flight += 1;
        }
        if self.network.len() < MAX_ENTRIES {
            self.index.insert(id.to_owned(), self.network.len());
            self.network.push(NetworkRequest {
                method: p["request"]["method"].as_str().unwrap_or("GET").to_owned(),
                url: UntrustedText::new(redact_url(p["request"]["url"].as_str().unwrap_or(""))),
                resource_type: rtype.to_owned(),
                status: None,
                error: None,
                duration_ms: None,
                started: ts,
                counted,
            });
        }
    }

    fn finish(&mut self, p: &Value, error: Option<String>) {
        let ts = p["timestamp"].as_f64().unwrap_or(0.0);
        let Some(r) = self.record(p) else { return };
        r.duration_ms = Some(ms_between(r.started, ts));
        r.error = error.or(r.error.take()); // keep a policy block
        if std::mem::take(&mut r.counted) {
            self.in_flight = self.in_flight.saturating_sub(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ev(method: &str, params: Value) -> CdpEvent {
        CdpEvent {
            method: method.into(),
            params,
            session_id: None,
        }
    }

    #[test]
    fn console_and_exception_are_redacted_and_levelled() {
        let mut l = Logs::default();
        l.handle(&ev("Runtime.consoleAPICalled", json!({"type":"error","args":[{"type":"string","value":"boom sk-ant-abcdefgh1234"},{"type":"number","value":42}]})));
        l.handle(&ev(
            "Runtime.consoleAPICalled",
            json!({"type":"warning","args":[]}),
        ));
        l.handle(&ev("Runtime.exceptionThrown", json!({"exceptionDetails":{"text":"Uncaught","lineNumber":7,"url":"http://h/?token=abc","exception":{"description":"Error: x"}}})));
        assert_eq!(l.console[0].level, "error");
        assert_eq!(
            l.console[0].text.expose(),
            "boom [REDACTED:anthropic-key] 42"
        );
        assert_eq!(l.console[1].level, "warning");
        assert_eq!(l.exceptions[0].text.expose(), "Error: x");
        assert_eq!(l.exceptions[0].line, Some(7));
        assert!(!l.exceptions[0].url.expose().contains("abc"));
    }

    #[test]
    fn network_lifecycle_redirect_failure_and_in_flight() {
        let mut l = Logs::default();
        let sent = |id: &str, url: &str, ts: f64, redirect: bool| {
            let mut p = json!({"requestId":id,"timestamp":ts,"type":"Document","request":{"url":url,"method":"GET"}});
            if redirect {
                p["redirectResponse"] = json!({"status":302});
            }
            ev("Network.requestWillBeSent", p)
        };
        l.handle(&sent("1", "http://h/a?api_key=zzz", 1.0, false));
        l.handle(&sent("1", "http://h/b", 1.1, true));
        assert_eq!(l.in_flight, 1, "redirect keeps one slot");
        l.handle(&ev(
            "Network.responseReceived",
            json!({"requestId":"1","response":{"status":404}}),
        ));
        l.handle(&ev(
            "Network.loadingFinished",
            json!({"requestId":"1","timestamp":1.5}),
        ));
        assert_eq!(l.in_flight, 0);
        assert_eq!(
            (l.network[0].status, l.network[1].status),
            (Some(302), Some(404))
        );
        assert!(!l.network[0].url.expose().contains("zzz"));
        assert_eq!(l.network[1].duration_ms, Some(400));
        assert!(l.network[1].is_failure());

        l.handle(&sent("2", "http://h/c", 2.0, false));
        l.handle(&ev(
            "Network.loadingFailed",
            json!({"requestId":"2","timestamp":2.2,"errorText":"net::ERR_CONNECTION_REFUSED"}),
        ));
        assert_eq!(
            l.network[2].error.as_deref(),
            Some("net::ERR_CONNECTION_REFUSED")
        );
        assert_eq!(l.in_flight, 0);

        let mut es = sent("3", "http://h/stream", 3.0, false);
        es.params["type"] = json!("EventSource");
        l.handle(&es);
        assert_eq!(l.in_flight, 0, "event streams do not block idle");
    }
}
