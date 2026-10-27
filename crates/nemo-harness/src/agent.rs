//! One agent's model loop (main agent or sub-agent): build context, stream, fold, run tools.

use std::collections::{BTreeMap, HashSet};

use futures_util::StreamExt;
use nemo_events::{Event, ToolStatus};
use nemo_providers::{
    Block, Content, Delta, ErrorKind, Message, MessageBuilder, ProviderError, Request, Role,
    StartBlock, StopReason, StreamEvent, ToolDef,
};
use serde_json::{Value, json};

use crate::harness::{AgentCtx, AgentEnd, AgentState, Harness, TaskRun};
use crate::util::{check_workspace, event_usage, lock, uuid};

pub(crate) enum StreamFail {
    Cancelled,
    Provider(ProviderError),
}

struct Streamed {
    msg: nemo_providers::AssistantMessage,
    /// Tool calls whose block never got a stop event (cut off).
    unstopped: HashSet<String>,
}

pub(crate) enum Problem {
    Truncated,
    Malformed(String),
}

pub(crate) struct Call {
    pub id: String,
    pub name: String,
    pub input: Option<Value>,
    pub problem: Option<Problem>,
}

fn delta_event(index: usize, d: &Delta) -> Option<Event> {
    let (kind, text) = match d {
        Delta::TextDelta { text } => ("text", text),
        Delta::ThinkingDelta { thinking } => ("thinking", thinking),
        Delta::InputJsonDelta { partial_json } => ("tool_input", partial_json),
        Delta::SignatureDelta { .. } | Delta::Unknown => return None,
    };
    Some(Event::ModelDelta {
        block_index: index as i64,
        kind: kind.into(),
        text: text.clone(),
    })
}

/// Event form of a content block: the thinking signature is provider-issued, never emitted.
fn block_value(b: &Block, tools: &crate::ToolBox) -> Value {
    let mut v = serde_json::to_value(b).unwrap_or(Value::Null);
    if let Some(o) = v.as_object_mut() {
        o.remove("signature");
    }
    if let Block::ToolUse {
        name,
        input: Some(i),
        ..
    } = b
    {
        v["input"] = tools.redact_input(name, i);
    }
    v
}

impl Harness {
    pub(crate) fn budget_hit(&self, task: &TaskRun, agent: &AgentCtx) -> Option<&'static str> {
        if let Some(k) = *lock(&task.hit) {
            return Some(k);
        }
        let found = {
            let c = lock(&task.counters);
            let b = &task.budget;
            let wall = task.started.elapsed().as_millis() as u64;
            if c.turns >= b.max_turns {
                Some(("max_turns", f64::from(b.max_turns), f64::from(c.turns)))
            } else if b.max_wall_ms.is_some_and(|m| wall >= m) {
                Some((
                    "max_wall_ms",
                    b.max_wall_ms.unwrap_or(0) as f64,
                    wall as f64,
                ))
            } else if self
                .config
                .max_total_tokens
                .is_some_and(|t| c.input + c.output >= t)
            {
                Some((
                    "max_tokens",
                    self.config.max_total_tokens.unwrap_or(0) as f64,
                    (c.input + c.output) as f64,
                ))
            } else if b.max_cost_usd.is_some_and(|m| c.cost >= m) {
                Some(("max_cost_usd", b.max_cost_usd.unwrap_or(0.0), c.cost))
            } else {
                None
            }
        };
        found.map(|(k, limit, used)| {
            self.mark_budget(task, agent, k, limit, used);
            k
        })
    }

    /// Records the first exceeded budget and emits `budget.exceeded` once.
    pub(crate) fn mark_budget(
        &self,
        task: &TaskRun,
        agent: &AgentCtx,
        kind: &'static str,
        limit: f64,
        used: f64,
    ) {
        let first = {
            let mut h = lock(&task.hit);
            h.is_none() && h.replace(kind).is_none()
        };
        if first {
            self.emit(
                task,
                agent,
                None,
                Event::BudgetExceeded {
                    budget: kind.into(),
                    limit,
                    used,
                },
            );
        }
    }

    async fn stream_once(
        &self,
        task: &TaskRun,
        agent: &AgentCtx,
        turn: &str,
        req: &Request,
    ) -> Result<Streamed, StreamFail> {
        let mut stream = self.provider.stream(req, task.cancel.clone());
        let mut builder = MessageBuilder::new();
        let mut open: BTreeMap<usize, Option<String>> = BTreeMap::new();
        // Tool-call blocks whose streamed arguments must not be emitted (they may hold secrets
        // and cannot be redacted before they are complete; `tool.requested` carries the redacted input).
        let mut masked: HashSet<usize> = HashSet::new();
        loop {
            let next = tokio::select! {
                biased;
                () = task.cancel.cancelled() => return Err(StreamFail::Cancelled),
                n = stream.next() => n,
            };
            let Some(item) = next else { break };
            let ev = item.map_err(StreamFail::Provider)?;
            match &ev {
                StreamEvent::ContentBlockStart {
                    index,
                    content_block: StartBlock::ToolUse { id, name, .. },
                } => {
                    open.insert(*index, id.clone());
                    // An unnamed block is unknown, so treated as masked.
                    if name.as_deref().is_none_or(|n| self.tools.masks_input(n)) {
                        masked.insert(*index);
                    }
                }
                StreamEvent::ContentBlockStop { index } => {
                    open.remove(index);
                    masked.remove(index);
                }
                StreamEvent::ContentBlockDelta { index, delta } => {
                    let hide =
                        masked.contains(index) && matches!(delta, Delta::InputJsonDelta { .. });
                    if let Some(e) = delta_event(*index, delta).filter(|_| !hide) {
                        self.emit(task, agent, Some(turn), e);
                    }
                }
                _ => {}
            }
            builder.push(&ev).map_err(StreamFail::Provider)?;
        }
        let msg = builder.finish().map_err(StreamFail::Provider)?;
        Ok(Streamed {
            msg,
            unstopped: open.into_values().flatten().collect(),
        })
    }

    /// Adds the response usage to the task counters; returns the event form.
    fn account(&self, task: &TaskRun, u: &nemo_providers::Usage) -> nemo_events::Usage {
        let mut c = lock(&task.counters);
        c.turns += 1;
        let (i, o) = (u.input_tokens.unwrap_or(0), u.output_tokens.unwrap_or(0));
        c.input += i;
        c.output += o;
        c.cache_read += u.cache_read_input_tokens.unwrap_or(0);
        c.cache_create += u.cache_creation_input_tokens.unwrap_or(0);
        let cost = self
            .config
            .pricing
            .map(|p| (i as f64 * p.input_per_mtok + o as f64 * p.output_per_mtok) / 1_000_000.0);
        c.cost += cost.unwrap_or(0.0);
        event_usage(u, cost)
    }

    fn tool_defs(&self, agent: &AgentCtx) -> Vec<ToolDef> {
        self.tools
            .specs()
            .into_iter()
            .filter(|s| agent.allowed.as_ref().is_none_or(|a| a.contains(&s.name)))
            .map(|s| ToolDef {
                name: s.name,
                description: Some(s.description),
                input_schema: s.input_schema,
            })
            .collect()
    }

    pub(crate) async fn run_agent(
        &self,
        task: &TaskRun,
        agent: &AgentCtx,
        st: &mut AgentState,
    ) -> AgentEnd {
        let mut iteration = 0i64;
        let mut recoveries = 0u32;
        loop {
            if task.cancel.is_cancelled() {
                return AgentEnd::Cancelled;
            }
            if self.sink.is_broken() {
                return AgentEnd::Internal("event_sink_failed".into());
            }
            if let Err(why) = check_workspace(&task.workspace) {
                return AgentEnd::UnsafeWorkspace(why);
            }
            if let Some(kind) = self.budget_hit(task, agent) {
                return AgentEnd::Budget(kind);
            }
            let built = match self.context.build(&st.messages, agent.prompt.as_deref()) {
                Ok(b) => b,
                Err(e) => return AgentEnd::Internal(format!("context_error: {e}")),
            };
            let max_output = task
                .caps
                .max_output
                .map_or(self.config.max_output_tokens, |m| {
                    self.config
                        .max_output_tokens
                        .min(m.min(u64::from(u32::MAX)) as u32)
                });
            let req = Request {
                model: self.config.model.clone(),
                max_tokens: max_output,
                system: Some(built.system),
                messages: built.messages,
                tools: self.tool_defs(agent),
                temperature: None,
                thinking: None,
                reasoning_effort: None,
            };
            let turn = uuid();
            iteration += 1;
            let t0 = std::time::Instant::now();
            self.emit(
                task,
                agent,
                Some(&turn),
                Event::TurnStarted {
                    model: self.config.model.clone(),
                    provider: self.provider.id().to_string(),
                    iteration,
                },
            );

            let streamed = match self.stream_once(task, agent, &turn, &req).await {
                Ok(s) => s,
                Err(StreamFail::Cancelled) => return AgentEnd::Cancelled,
                Err(StreamFail::Provider(e)) => {
                    if task.cancel.is_cancelled() || e.kind == ErrorKind::Cancelled {
                        return AgentEnd::Cancelled;
                    }
                    self.emit(
                        task,
                        agent,
                        Some(&turn),
                        Event::ModelError {
                            status: e.status.map(i64::from),
                            message: e.message().to_string(),
                            retryable: e.retryable(),
                            context_overflow: e.context_overflow(),
                        },
                    );
                    if e.context_overflow() && !st.compacted {
                        st.compacted = true;
                        match self.context.compact(&st.messages).await {
                            Ok(c) => {
                                self.emit(
                                    task,
                                    agent,
                                    None,
                                    Event::ContextCompacted {
                                        trigger: "auto".into(),
                                        summary_ref: c.summary_ref,
                                        tokens_before: c.tokens_before,
                                    },
                                );
                                st.messages = c.messages;
                                continue;
                            }
                            Err(why) => {
                                return AgentEnd::ModelError {
                                    blocked: false,
                                    reason: format!("context_overflow: compaction failed: {why}"),
                                };
                            }
                        }
                    }
                    // Only a failure after the first byte is retried here: before that the
                    // provider layer already used its retry budget.
                    let transient = matches!(
                        e.kind,
                        ErrorKind::RateLimit
                            | ErrorKind::Overloaded
                            | ErrorKind::Server
                            | ErrorKind::Network
                            | ErrorKind::InvalidResponse
                    );
                    if e.mid_stream && transient && recoveries < self.config.stream_recoveries {
                        recoveries += 1;
                        let delay = self.config.recovery_delay;
                        self.emit(
                            task,
                            agent,
                            Some(&turn),
                            Event::ModelRetry {
                                attempt: i64::from(recoveries),
                                max: i64::from(self.config.stream_recoveries),
                                error: e.message().to_string(),
                                delay_ms: delay.as_millis() as u64,
                            },
                        );
                        tokio::select! {
                            () = tokio::time::sleep(delay) => {}
                            () = task.cancel.cancelled() => return AgentEnd::Cancelled,
                        }
                        continue;
                    }
                    return AgentEnd::ModelError {
                        blocked: e.kind == ErrorKind::Auth,
                        reason: format!("model_error:{}: {}", e.kind.as_str(), e.message()),
                    };
                }
            };
            recoveries = 0;

            let Streamed { mut msg, unstopped } = streamed;
            let usage = self.account(task, &msg.usage);
            let has_tools = msg
                .content
                .iter()
                .any(|b| matches!(b, Block::ToolUse { .. }));
            let stop_str = msg.stop_reason.clone().map_or_else(
                || {
                    if has_tools {
                        "tool_use".to_string()
                    } else {
                        "end_turn".to_string()
                    }
                },
                String::from,
            );
            self.emit(
                task,
                agent,
                Some(&turn),
                Event::AssistantMessage {
                    message_id: msg.id.clone(),
                    content: msg
                        .content
                        .iter()
                        .map(|b| block_value(b, &self.tools))
                        .collect(),
                    stop_reason: stop_str.clone(),
                    usage: Some(usage.clone()),
                },
            );
            self.emit(
                task,
                agent,
                Some(&turn),
                Event::TurnCompleted {
                    stop_reason: stop_str,
                    usage: Some(usage),
                    duration_ms: t0.elapsed().as_millis() as u64,
                },
            );

            // Fold into history and collect the calls. A call without valid arguments stays in
            // history with `{}` (providers reject a tool_use without input) and gets an error result.
            let max_tokens_hit = msg.stop_reason == Some(StopReason::MaxTokens);
            let mut calls = vec![];
            let mut final_text = String::new();
            for b in &mut msg.content {
                match b {
                    Block::Text { text } => final_text.push_str(text),
                    Block::ToolUse { id, name, input } => {
                        let bad = msg
                            .invalid_tool_inputs
                            .iter()
                            .find(|i| i.tool_use_id == *id);
                        let problem = if unstopped.contains(id.as_str())
                            || (bad.is_some() && max_tokens_hit)
                        {
                            Some(Problem::Truncated)
                        } else {
                            bad.map(|i| Problem::Malformed(i.error.clone()))
                        };
                        calls.push(Call {
                            id: id.clone(),
                            name: name.clone(),
                            input: if problem.is_some() {
                                None
                            } else {
                                input.clone()
                            },
                            problem,
                        });
                        // History (returned to the caller, possibly persisted) keeps no secrets.
                        *input = Some(match input.take() {
                            Some(i) => self.tools.redact_input(name, &i),
                            None => json!({}),
                        });
                    }
                    _ => {}
                }
            }
            if !msg.content.is_empty() {
                st.messages.push(Message {
                    role: Role::Assistant,
                    content: Content::Blocks(msg.content.clone()),
                });
            }

            if calls.is_empty() {
                match msg.stop_reason {
                    Some(StopReason::Refusal) => return AgentEnd::Refusal,
                    Some(StopReason::PauseTurn) => continue,
                    _ => return AgentEnd::Finished(final_text),
                }
            }

            let outcome = self.run_batch(task, agent, &turn, calls).await;
            st.messages.push(Message {
                role: Role::User,
                content: Content::Blocks(outcome.blocks),
            });
            if task.cancel.is_cancelled() {
                return AgentEnd::Cancelled;
            }
            if let Some(kind) = *lock(&task.hit) {
                return AgentEnd::Budget(kind);
            }
            st.recent.push_back(outcome.fingerprint);
            while st.recent.len() > self.config.no_progress_window.max(1) {
                st.recent.pop_front();
            }
            if st.recent.len() == self.config.no_progress_window.max(1)
                && st.recent.iter().all(|f| *f == st.recent[0])
            {
                return AgentEnd::NoProgress {
                    approval_unavailable: outcome.all_unavailable,
                };
            }
        }
    }
}

pub(crate) fn tool_status_for(is_error: bool, soft: bool) -> ToolStatus {
    match (is_error, soft) {
        (false, _) => ToolStatus::Ok,
        (true, true) => ToolStatus::SoftError,
        (true, false) => ToolStatus::Error,
    }
}
