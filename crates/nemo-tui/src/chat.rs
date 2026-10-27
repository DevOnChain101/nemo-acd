//! Event stream -> conversation items of the agent pane. The UI keeps no truth of its own: every
//! row is derived from an `Envelope`. Items mutate in place while streaming, so every mutation
//! bumps `Item::rev` (the renderer caches rows per item, see `nemo_tui_render::agent::Item`).

use std::collections::HashMap;

use nemo_events::{Envelope, Event, TaskStatus, ToolStatus};
use nemo_tui_render::agent::{Agent, Item};
use serde_json::Value;

use crate::sanitize::{clean_input_lines, clean_line, clean_text};

/// Running totals the status line shows (`BudgetUsage` only exists at task end).
#[derive(Clone, Debug, Default)]
pub struct Totals {
    pub turns: u32,
    pub tool_calls: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verification {
    /// passed / failed / unverified / blocked
    pub status: String,
    pub summary: String,
}

#[derive(Default)]
pub struct Chat {
    pub agent: Agent,
    tools: HashMap<String, usize>,
    text: Option<usize>,
    think: Option<usize>,
    turn_text: bool,
    pub totals: Totals,
    pub verification: Option<Verification>,
    /// Task counters start fresh with every task.
    pub task_started: Option<i64>,
    /// Startup notes whose `Notice` event has not come through the event channel yet.
    /// `(tone, text)`: a later notice with the same text but another severity is a different event.
    startup: Vec<(String, String)>,
}

fn touch(it: &mut Item) {
    it.rev += 1;
}

fn note(text: impl Into<String>, tone: &str) -> Item {
    item! {
        kind: "note".into(),
        text: clean_text(&text.into()),
        tone: tone.into(),
    }
}

impl Chat {
    pub fn push(&mut self, it: Item) -> usize {
        self.agent.items.push(it);
        self.agent.items.len() - 1
    }

    pub fn note(&mut self, text: impl Into<String>, tone: &str) {
        self.push(note(text, tone));
    }

    /// Startup notes are shown now; the same `Notice` event arriving later is not shown again.
    pub fn startup_note(&mut self, text: &str) {
        let t = clean_line(text);
        self.startup.push(("warn".into(), t.clone()));
        self.note(t, "warn");
    }

    fn take_startup(&mut self, tone: &str, text: &str) -> bool {
        let t = clean_line(text);
        self.startup
            .iter()
            .position(|(k, s)| k == tone && *s == t)
            .map(|i| self.startup.remove(i))
            .is_some()
    }

    fn finish_streams(&mut self, now: i64) {
        if let Some(i) = self.text.take() {
            let it = &mut self.agent.items[i];
            it.streaming = false;
            touch(it);
        }
        if let Some(i) = self.think.take() {
            let it = &mut self.agent.items[i];
            it.streaming = false;
            it.end = Some(now);
            touch(it);
        }
    }

    /// `now` is the UI clock (ms), the same one the renderer gets.
    pub fn apply(&mut self, env: &Envelope, now: i64) {
        let sub = env.agent_id.clone();
        match &env.event {
            Event::TaskStarted { .. } => {
                self.task_started = Some(now);
                self.agent.turn_start = now;
                self.agent.turn_tokens = 0;
                self.agent.status = "running".into();
                self.agent.interrupting = false;
                self.verification = None;
                self.totals = Totals::default();
            }
            Event::UserMessage {
                text, display_text, ..
            } => {
                let t = display_text.as_deref().unwrap_or(text);
                self.push(item! {
                    kind: "user".into(),
                    text: clean_text(t),
                });
            }
            Event::TurnStarted { .. } => {
                self.turn_text = false;
                self.totals.turns += 1;
            }
            Event::ModelDelta { kind, text, .. } if sub.is_none() => match kind.as_str() {
                "text" => {
                    let i = match self.text {
                        Some(i) => i,
                        None => {
                            let i = self.push(item! {
                                kind: "text".into(),
                                streaming: true,
                            });
                            self.text = Some(i);
                            i
                        }
                    };
                    self.turn_text = true;
                    let it = &mut self.agent.items[i];
                    it.text.push_str(&clean_text(text));
                    touch(it);
                }
                "thinking" => {
                    let i = match self.think {
                        Some(i) => i,
                        None => {
                            let i = self.push(item! {
                                kind: "thinking".into(),
                                streaming: true,
                                start: Some(now),
                            });
                            self.think = Some(i);
                            i
                        }
                    };
                    let it = &mut self.agent.items[i];
                    it.text.push_str(&clean_text(text));
                    touch(it);
                }
                _ => {}
            },
            Event::AssistantMessage { content, .. } if sub.is_none() => {
                self.finish_streams(now);
                if !self.turn_text {
                    for b in content {
                        if b.get("type").and_then(Value::as_str) == Some("text")
                            && let Some(t) = b.get("text").and_then(Value::as_str)
                            && !t.trim().is_empty()
                        {
                            self.push(item! {
                                kind: "text".into(),
                                text: clean_text(t),
                            });
                        }
                    }
                }
            }
            Event::TurnCompleted { usage, .. } if sub.is_none() => {
                self.finish_streams(now);
                if let Some(u) = usage {
                    self.totals.input_tokens += u.input_tokens;
                    self.totals.output_tokens += u.output_tokens;
                    self.agent.turn_tokens += u.output_tokens;
                    if let Some(c) = u.cost_usd {
                        *self.totals.cost_usd.get_or_insert(0.0) += c;
                    }
                }
            }
            Event::ModelRetry {
                attempt,
                max,
                error,
                ..
            } => self.note(format!("model retry {attempt}/{max}: {error}"), "warn"),
            Event::ModelError { message, .. } => {
                self.note(format!("model error: {message}"), "error")
            }
            Event::ContextCompacted { trigger, .. } => {
                self.note(format!("context compacted ({trigger})"), "")
            }
            Event::ToolRequested {
                tool_call_id,
                name,
                input,
                ..
            } => {
                self.finish_streams(now);
                self.totals.tool_calls += 1;
                let name = clean_line(name);
                let name = match &sub {
                    Some(a) => format!(
                        "@{}:{name}",
                        clean_line(&a.chars().take(6).collect::<String>())
                    ),
                    None => name,
                };
                let i = self.push(item! {
                    kind: "tool".into(),
                    name,
                    input: clean_input_lines(input),
                    status: "running".into(),
                    start: Some(now),
                });
                self.tools.insert(tool_call_id.clone(), i);
            }
            Event::ToolCompleted {
                tool_call_id,
                status,
                preview,
                ..
            } => {
                if let Some(&i) = self.tools.get(tool_call_id) {
                    let it = &mut self.agent.items[i];
                    it.status = match status {
                        ToolStatus::Ok => "done",
                        ToolStatus::Interrupted => "interrupted",
                        _ => "error",
                    }
                    .into();
                    it.soft = *status == ToolStatus::SoftError;
                    it.result = clean_text(preview);
                    it.end = Some(now);
                    touch(it);
                }
            }
            Event::ToolApprovalResolved {
                decision, reviewer, ..
            } if reviewer != "user" || decision == "expired" => {
                self.note(format!("approval {decision} ({reviewer})"), "warn");
            }
            Event::VerificationCompleted {
                status, summary, ..
            } => {
                self.verification = Some(Verification {
                    status: clean_line(status),
                    summary: clean_text(summary),
                });
                let tone = match status.as_str() {
                    "passed" => "goal",
                    "failed" => "error",
                    _ => "warn",
                };
                self.note(format!("verification {status}: {summary}"), tone);
            }
            Event::BudgetExceeded {
                budget,
                limit,
                used,
            } => self.note(
                format!("budget exceeded: {budget} {used} / {limit}"),
                "warn",
            ),
            Event::SubagentStarted(i) => self.note(
                format!("agent {} started ({})", i.agent_id, i.agent_type),
                "",
            ),
            Event::SubagentStopped(i) => self.note(
                format!("agent {} stopped ({})", i.agent_id, i.agent_type),
                "",
            ),
            Event::ModelChanged { model, .. } => {
                self.agent.model = clean_line(model);
                self.note(format!("model: {model}"), "");
            }
            // Already shown as a startup note: the event log keeps it, the pane shows it once.
            Event::Notice { level, text } => {
                let tone = match level.as_str() {
                    "error" => "error",
                    "warning" | "warn" => "warn",
                    _ => "",
                };
                if !self.take_startup(tone, text) {
                    self.note(text.clone(), tone);
                }
            }
            Event::TaskEnded {
                status,
                reason,
                usage,
                ..
            } => {
                self.finish_streams(now);
                for it in self.agent.items.iter_mut().rev() {
                    if it.kind == "text" {
                        it.is_final = *status == TaskStatus::Completed;
                        touch(it);
                        break;
                    }
                    if it.kind == "user" {
                        break;
                    }
                }
                // A task that is not `completed` is never shown as success.
                if *status == TaskStatus::Completed {
                    let ms = self.task_started.map_or(0, |t| now - t);
                    self.push(item! {
                        kind: "result".into(),
                        ok: true,
                        turns: u64::from(self.totals.turns),
                        ms,
                        cost: usage.cost_usd,
                    });
                } else {
                    let (word, tone) = match status {
                        TaskStatus::Failed => ("failed", "error"),
                        TaskStatus::Cancelled => ("cancelled", "warn"),
                        TaskStatus::Blocked => ("blocked", "warn"),
                        TaskStatus::Unverified => ("unverified", "warn"),
                        _ => ("ended", "warn"),
                    };
                    self.note(format!("task {word}: {reason}"), tone);
                }
                if matches!(status, TaskStatus::Unverified | TaskStatus::Blocked)
                    && self.verification.is_none()
                {
                    self.verification = Some(Verification {
                        status: if *status == TaskStatus::Blocked {
                            "blocked"
                        } else {
                            "unverified"
                        }
                        .into(),
                        summary: clean_text(reason),
                    });
                }
                self.agent.status = "idle".into();
                self.agent.interrupting = false;
                self.tools.clear();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_name_and_input_are_stored_without_escape_bytes() {
        let evil = "a\x1b]52;c;ZXZpbA==\x07b\x1b[6nc\u{9b}6n\u{202e}d\x08e";
        let mut chat = Chat::default();
        chat.apply(
            &Envelope {
                v: 1,
                seq: 1,
                ts: "2026-01-01T00:00:00.000Z".into(),
                session_id: "s".into(),
                task_id: None,
                turn_id: None,
                agent_id: None,
                event: Event::ToolRequested {
                    tool_call_id: "c".into(),
                    name: format!("T{evil}"),
                    input: json!({ evil: [evil, {"k": evil}], "n": 1 }),
                    input_truncated: false,
                },
            },
            0,
        );
        let it = chat.agent.items.last().unwrap();
        // JSON text escapes control chars as \uXXXX, so a surviving one shows up as that.
        let all = format!("{}{}", it.name, it.input);
        assert!(
            !it.name.chars().any(|c| c.is_control() || c == '\u{202e}'),
            "{all:?}"
        );
        for bad in ["\\u001b", "\\u009b", "\\u0007", "\\u0008", "\\u202e"] {
            assert!(!all.contains(bad), "{bad} in {all}");
        }
        assert!(it.name.starts_with('T'));
    }
}
