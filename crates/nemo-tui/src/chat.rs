//! Event stream -> conversation items of the agent pane. The UI keeps no truth of its own: every
//! row is derived from an `Envelope`. Items mutate in place while streaming, so every mutation
//! bumps `Item::rev` (the renderer caches rows per item, see `nemo_tui_render::agent::Item`).

use std::collections::HashMap;

use nemo_events::{Envelope, Event, TaskStatus, ToolStatus};
use nemo_tui_render::agent::{Agent, Item};
use nemo_tui_render::model::parse_ts;
use serde_json::Value;

use crate::sanitize::{clean_input_lines, clean_line, clean_text};
use crate::status::ContextStatus;

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
    /// Event time (ms) of the first delta of the open thinking block.
    think_t0: Option<i64>,
    turn_text: bool,
    pub totals: Totals,
    pub verification: Option<Verification>,
    pub ctx: ContextStatus,
    /// Task counters start fresh with every task.
    pub task_started: Option<i64>,
    /// Startup notes whose `Notice` event has not come through the event channel yet.
    /// `(tone, text)`: a later notice with the same text but another severity is a different event.
    startup: Vec<(String, String)>,
}

/// Most live output text kept per running tool row (the tail).
const MAX_LIVE: usize = 256 * 1024;

fn touch(it: &mut Item) {
    it.rev += 1;
}

/// `tool.completed.diff` with every text made safe to draw (file content is untrusted).
fn clean_diff(d: &Value) -> Value {
    match d {
        Value::String(s) => Value::String(clean_text(s)),
        Value::Array(a) => Value::Array(a.iter().map(clean_diff).collect()),
        Value::Object(o) => {
            Value::Object(o.iter().map(|(k, v)| (k.clone(), clean_diff(v))).collect())
        }
        v => v.clone(),
    }
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
                // `Some("")`: a note from the harness to the model, not something typed
                if !t.is_empty() {
                    self.push(item! {
                        kind: "user".into(),
                        text: clean_text(t),
                    });
                }
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
                            self.think_t0 = None;
                            i
                        }
                    };
                    let it = &mut self.agent.items[i];
                    it.text.push_str(&clean_text(text));
                    // the block's duration is first to last delta, so a replay shows the real one
                    if let Some(t) = parse_ts(&env.ts) {
                        it.ms = t - *self.think_t0.get_or_insert(t);
                    }
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
            Event::ContextUsage {
                used_tokens,
                window_tokens,
                percent,
                stale,
                ..
            } if sub.is_none() => {
                self.ctx
                    .on_usage(*used_tokens, *window_tokens, Some(*percent), *stale)
            }
            Event::ContextCompacting { .. } if sub.is_none() => self.ctx.on_compacting(),
            Event::ContextCompactionFailed { reason, error } => {
                self.ctx.on_compaction_failed();
                self.note(
                    format!("context compaction failed ({reason}): {error}"),
                    "warn",
                );
            }
            Event::ContextCompacted {
                trigger,
                tokens_before,
                before_tokens,
                after_tokens,
                window_tokens,
                ..
            } => {
                self.note(format!("context compacted ({trigger})"), "");
                if sub.is_none() {
                    // logs from before the numbers existed carry only `tokens_before`
                    let before = if *before_tokens > 0 {
                        *before_tokens
                    } else {
                        tokens_before.unwrap_or(0)
                    };
                    self.ctx
                        .on_compacted(before, *after_tokens, *window_tokens, now);
                }
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
            Event::ToolOutput {
                tool_call_id, text, ..
            } => {
                if let Some(&i) = self.tools.get(tool_call_id) {
                    let it = &mut self.agent.items[i];
                    it.output.push_str(&clean_text(text));
                    if it.output.len() > MAX_LIVE {
                        // the live view is the tail; the whole result arrives with tool.completed
                        let mut cut = it.output.len() - MAX_LIVE;
                        while !it.output.is_char_boundary(cut) {
                            cut += 1;
                        }
                        it.output.drain(..cut);
                    }
                    touch(it);
                }
            }
            Event::ToolCompleted {
                tool_call_id,
                status,
                preview,
                output,
                output_truncated,
                output_bytes,
                result_ref,
                diff,
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
                    // the whole model-facing text; the 200-char preview only for older logs
                    it.result = clean_text(output.as_deref().unwrap_or(preview));
                    if *output_truncated {
                        let whole = match result_ref {
                            Some(a) => format!(
                                "full output ({output_bytes} bytes): artifact {}",
                                clean_line(&a.path)
                            ),
                            None => format!("the whole ({output_bytes} bytes) could not be saved"),
                        };
                        it.result = format!("[output truncated · {whole}]\n{}", it.result);
                    }
                    if let Some(d) = diff {
                        it.diff = clean_diff(d);
                    }
                    it.output.clear();
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
            Event::AutoVerdict {
                name,
                verdict,
                reason,
                ..
            } => self.note(
                format!("auto mode: {verdict} {name}: {reason}"),
                if verdict == "allow" { "" } else { "warn" },
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
                    origin: None,
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

    /// A stored session: thinking deltas 4.2 s apart, then an Edit with a `diff` and one without.
    #[test]
    fn replayed_session_shows_thinking_duration_and_line_numbered_diff() {
        let line = |seq: u32, ts: &str, ty: &str, payload: Value| {
            json!({"v": 1, "seq": seq, "ts": ts, "session_id": "s", "task_id": null,
                "turn_id": null, "type": ty, "payload": payload})
            .to_string()
        };
        let done = |id: &str, diff: Option<Value>| {
            let mut p = json!({"tool_call_id": id, "name": "Edit", "status": "ok", "is_error": false,
                "not_run": false, "duration_ms": 1, "preview": "p", "bytes": 1, "truncated": false});
            if let Some(d) = diff {
                p["diff"] = d;
            }
            p
        };
        let input = json!({"file_path": "src/a.rs", "old_string": "let a = 1;", "new_string": "let a = 2;"});
        let req = |id: &str| json!({"tool_call_id": id, "name": "Edit", "input": input, "input_truncated": false});
        let diff = json!({"path": "src/a.rs", "kind": "update", "added": 1, "removed": 1, "truncated": false,
            "hunks": [{"old_start": 41, "old_lines": 1, "new_start": 41, "new_lines": 1,
                "lines": [{"tag": "-", "old_no": 41, "text": "let a = 1;"}, {"tag": "+", "new_no": 41, "text": "let a = 2;\u{1b}[31m"}]}]});
        let jsonl = [
            line(
                1,
                "2026-01-01T00:00:00.000Z",
                "model.delta",
                json!({"block_index": 0, "kind": "thinking", "text": "hmm "}),
            ),
            line(
                2,
                "2026-01-01T00:00:04.200Z",
                "model.delta",
                json!({"block_index": 0, "kind": "thinking", "text": "ok"}),
            ),
            line(3, "2026-01-01T00:00:09.000Z", "tool.requested", req("a")),
            line(
                4,
                "2026-01-01T00:00:10.000Z",
                "tool.completed",
                done("a", Some(diff)),
            ),
            line(5, "2026-01-01T00:00:11.000Z", "tool.requested", req("b")),
            line(
                6,
                "2026-01-01T00:00:12.000Z",
                "tool.completed",
                done("b", None),
            ),
        ]
        .join("\n");
        let mut chat = Chat::default();
        for rec in nemo_events::EventReader::new(jsonl.as_bytes()) {
            let nemo_events::Record::Known(e) = rec.unwrap() else {
                panic!("known")
            };
            chat.apply(&e, parse_ts(&e.ts).unwrap());
        }
        let (lines, _) =
            nemo_tui_render::agent::transcript_rows(&chat.agent, 80, 40, 0, 0, false, 0);
        let text: Vec<String> = lines
            .iter()
            .map(|l| nemo_tui_base::text::strip_ansi(l).trim_end().to_string())
            .collect();
        let all = text.join("\n");
        assert!(all.contains("✻ Thinking · 4.2s"), "{all}");
        assert!(all.contains("│ hmm ok"), "{all}");
        assert!(all.contains("Updated src/a.rs — +1 −1 (line 41)"), "{all}");
        assert!(
            all.contains("41     − let a = 1;") || all.contains("41    − let a = 1;"),
            "{all}"
        );
        assert!(
            !all.contains('\u{1b}'),
            "escape bytes must not reach the screen"
        );
        assert!(
            all.contains("(line numbers unavailable)"),
            "older event without diff: {all}"
        );
    }

    fn env(seq: u64, event: Event) -> Envelope {
        Envelope {
            v: 1,
            seq,
            ts: "2026-01-01T00:00:00.000Z".into(),
            session_id: "s".into(),
            task_id: None,
            turn_id: None,
            agent_id: None,
            event,
        }
    }

    fn requested(id: &str, name: &str) -> Event {
        Event::ToolRequested {
            tool_call_id: id.into(),
            name: name.into(),
            input: json!({"command": "x"}),
            input_truncated: false,
            origin: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn completed(id: &str, preview: &str, output: Option<&str>, cut: bool, aref: bool) -> Event {
        Event::ToolCompleted {
            tool_call_id: id.into(),
            name: "Bash".into(),
            status: ToolStatus::Ok,
            is_error: false,
            not_run: false,
            duration_ms: 1,
            preview: preview.into(),
            bytes: 9,
            truncated: false,
            result_ref: aref.then(|| nemo_events::ArtifactRef {
                artifact_id: "a".into(),
                path: "terminal/proc_1-ab.log".into(),
                size: 9,
                sha256: "0".into(),
                mime: "text/plain".into(),
            }),
            origin: None,
            output: output.map(Into::into),
            output_truncated: cut,
            output_bytes: 9_000_000,
            diff: None,
        }
    }

    #[test]
    fn live_output_is_the_running_rows_tail_and_the_completed_row_uses_the_whole_output() {
        let mut chat = Chat::default();
        chat.apply(&env(1, requested("c", "Bash")), 0);
        for (i, t) in ["one\ntw", "o\nthree\n"].iter().enumerate() {
            chat.apply(
                &env(
                    2 + i as u64,
                    Event::ToolOutput {
                        tool_call_id: "c".into(),
                        stream: "merged".into(),
                        text: (*t).into(),
                        seq: 1 + i as u64,
                        origin: None,
                    },
                ),
                0,
            );
        }
        let it = chat.agent.items.last().unwrap();
        assert_eq!(
            (it.status.as_str(), it.output.as_str()),
            ("running", "one\ntwo\nthree\n")
        );
        let whole = "one\ntwo\nthree\nfour";
        chat.apply(
            &env(4, completed("c", "one\ntwo...", Some(whole), false, false)),
            5,
        );
        let it = chat.agent.items.last().unwrap();
        assert_eq!(
            (it.status.as_str(), it.result.as_str(), it.output.as_str()),
            ("done", whole, "")
        );
    }

    #[test]
    fn a_cut_output_is_marked_with_its_artifact_path_never_silently() {
        let mut chat = Chat::default();
        chat.apply(&env(1, requested("c", "mcp__x__y")), 0);
        chat.apply(
            &env(2, completed("c", "p", Some("head ... tail"), true, true)),
            1,
        );
        let r = &chat.agent.items.last().unwrap().result;
        assert!(
            r.starts_with("[output truncated")
                && r.contains("terminal/proc_1-ab.log")
                && r.ends_with("head ... tail"),
            "{r}"
        );
        // no artifact could be written: still said, not hidden
        chat.apply(&env(3, requested("d", "mcp__x__y")), 0);
        chat.apply(
            &env(4, completed("d", "p", Some("head ... tail"), true, false)),
            1,
        );
        assert!(
            chat.agent
                .items
                .last()
                .unwrap()
                .result
                .contains("could not be saved")
        );
        // logs from before `output` existed still show their preview
        chat.apply(&env(5, requested("e", "Read")), 0);
        chat.apply(
            &env(6, completed("e", "old preview", None, false, false)),
            1,
        );
        assert_eq!(chat.agent.items.last().unwrap().result, "old preview");
    }

    #[test]
    fn a_note_to_the_model_is_not_a_user_row() {
        let mut chat = Chat::default();
        let msg = |t: &str, d: Option<&str>| {
            env(
                1,
                Event::UserMessage {
                    text: t.into(),
                    display_text: d.map(Into::into),
                    images: vec![],
                },
            )
        };
        chat.apply(&msg("[Nemo note] analyse", Some("")), 0);
        assert!(chat.agent.items.is_empty());
        chat.apply(
            &msg("<user-bash-input>ls</user-bash-input>", Some("!ls")),
            0,
        );
        assert_eq!(chat.agent.items[0].text, "!ls");
    }
}
