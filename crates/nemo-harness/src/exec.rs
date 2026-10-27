//! Tool pipeline (tools.md §3): parse -> validate -> policy/approval -> PreToolUse -> budget ->
//! execute -> PostToolUse -> result. Also the harness-run tools: `Task`, `AskUserQuestion`,
//! `ExitPlanMode`.

use std::collections::BTreeSet;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::Ordering;
use std::time::Instant;

use futures_util::future::join_all;
use nemo_events::{Event, Todo, ToolStatus};
use nemo_hooks::{Decision as HookDecision, HookEvent};
use nemo_permissions::{Decision, Mode, Suggestion, Verdict};
use nemo_providers::{Block, Content, Message, Role};
use nemo_tools::{
    Block as ToolBlock, Content as ToolContent, NeedsUser, SideEffectClass, ToolRuntime, ToolSpec,
};
use nemo_verification::{Check, CheckKind, DEFAULT_TIMEOUT_MS, Declarer};
use serde_json::{Value, json};

use crate::agent::{Call, Problem};
use crate::agents::stricter;
use crate::context::wrap_tool_output;
use crate::harness::{AgentCtx, AgentEnd, AgentState, Harness, TaskRun};
use crate::toolbox::validate;
use crate::traits::{Approval, ApprovalDecision, ApprovalRequest, DENY_MESSAGE, UserReply};
use crate::util::{cap_text, lock, preview, sanitize_for_display, uuid};

pub(crate) struct BatchOutcome {
    pub blocks: Vec<Block>,
    pub fingerprint: u64,
    pub all_unavailable: bool,
}

/// What the model sees for one call, plus what the loop needs to know about it.
pub(crate) struct Res {
    pub content: Value,
    pub text: String,
    pub is_error: bool,
    pub status: ToolStatus,
    /// Denied because nobody could approve.
    pub unavailable: bool,
}

struct Denial {
    text: String,
    unavailable: bool,
    /// Hook output goes to the model, never into the event log (events.md §5.1).
    from_hook: bool,
}

enum Plan {
    Done(Res),
    Run { input: Value, spec: ToolSpec },
    Special { input: Value },
}

type Pending = (usize, String, String, Value, ToolSpec);

fn side_effect_str(s: SideEffectClass) -> &'static str {
    match s {
        SideEffectClass::Read => "read",
        SideEffectClass::Write => "write",
        SideEffectClass::Execute => "execute",
        SideEffectClass::External => "external",
    }
}

fn suggestion_json(s: &Suggestion) -> Value {
    match s {
        Suggestion::AddRules { rules, behavior } => json!({
            "kind": "add_rules",
            "behavior": format!("{behavior:?}").to_lowercase(),
            "rules": rules.iter().map(|r| json!({"tool_name": r.tool_name, "rule_content": r.rule_content})).collect::<Vec<_>>(),
        }),
        Suggestion::SetMode(m) => json!({"kind": "set_mode", "mode": m.as_str()}),
    }
}

/// What the approver sees of the call. Never truncated: a cut command could hide the part that
/// matters. Callers pass the redacted input.
fn summarize(input: &Value) -> String {
    let s = ["command", "file_path", "path", "url", "pattern"]
        .iter()
        .find_map(|k| input.get(*k).and_then(Value::as_str))
        .map_or_else(|| input.to_string(), String::from);
    sanitize_for_display(&s)
}

impl Harness {
    fn agent_spec(&self, agent: &AgentCtx, name: &str) -> Option<ToolSpec> {
        self.tools
            .spec(name)
            .filter(|s| agent.allowed.as_ref().is_none_or(|a| a.contains(&s.name)))
    }

    /// Emits `tool.completed` and returns the model-visible result.
    #[allow(clippy::too_many_arguments)]
    fn complete(
        &self,
        task: &TaskRun,
        agent: &AgentCtx,
        turn: &str,
        id: &str,
        name: &str,
        t0: Option<Instant>,
        res: Res,
        not_run: bool,
        truncated: bool,
    ) -> Res {
        self.emit(
            task,
            agent,
            Some(turn),
            Event::ToolCompleted {
                tool_call_id: id.into(),
                name: name.into(),
                status: res.status,
                is_error: res.is_error,
                not_run,
                duration_ms: t0.map_or(0, |t| t.elapsed().as_millis() as u64),
                preview: preview(&res.text),
                bytes: res.text.len() as u64,
                truncated,
                result_ref: None,
            },
        );
        res
    }

    /// A harness-authored error (not tool output, so not wrapped as data).
    #[allow(clippy::too_many_arguments)]
    fn not_run(
        &self,
        task: &TaskRun,
        agent: &AgentCtx,
        turn: &str,
        id: &str,
        name: &str,
        status: ToolStatus,
        text: String,
        unavailable: bool,
    ) -> Res {
        let res = Res {
            content: Value::String(text.clone()),
            text,
            is_error: true,
            status,
            unavailable,
        };
        self.complete(task, agent, turn, id, name, None, res, true, false)
    }

    pub(crate) async fn run_batch(
        &self,
        task: &TaskRun,
        agent: &AgentCtx,
        turn: &str,
        calls: Vec<Call>,
    ) -> BatchOutcome {
        let mut slots: Vec<Option<Res>> = calls.iter().map(|_| None).collect();
        let mut group: Vec<Pending> = vec![];
        let parallel = task.caps.parallel_tools;
        for (i, call) in calls.iter().enumerate() {
            self.emit(
                task,
                agent,
                Some(turn),
                Event::ToolRequested {
                    tool_call_id: call.id.clone(),
                    name: call.name.clone(),
                    input: call
                        .input
                        .as_ref()
                        .map_or_else(|| json!({}), |i| self.tools.redact_input(&call.name, i)),
                    input_truncated: call.problem.is_some(),
                },
            );
            if task.cancel.is_cancelled() {
                let r = self.not_run(
                    task,
                    agent,
                    turn,
                    &call.id,
                    &call.name,
                    ToolStatus::Interrupted,
                    "Interrupted before this tool ran.".into(),
                    false,
                );
                slots[i] = Some(r);
                continue;
            }
            match self.plan_call(task, agent, turn, call).await {
                Plan::Done(r) => slots[i] = Some(r),
                Plan::Run { input, spec } => {
                    if parallel && spec.side_effect == SideEffectClass::Read {
                        group.push((i, call.id.clone(), call.name.clone(), input, spec));
                    } else {
                        self.flush(task, agent, turn, &mut group, &mut slots).await;
                        slots[i] = Some(
                            self.exec_call(task, agent, turn, &call.id, &call.name, input, &spec)
                                .await,
                        );
                    }
                }
                Plan::Special { input } => {
                    self.flush(task, agent, turn, &mut group, &mut slots).await;
                    slots[i] = Some(self.run_special(task, agent, turn, call, input).await);
                }
            }
        }
        self.flush(task, agent, turn, &mut group, &mut slots).await;

        let mut h = std::collections::hash_map::DefaultHasher::new();
        let mut blocks = vec![];
        let mut all_unavailable = !calls.is_empty();
        for (call, slot) in calls.iter().zip(slots) {
            let r = slot.unwrap_or_else(|| Res {
                content: Value::String("Tool did not run.".into()),
                text: "Tool did not run.".into(),
                is_error: true,
                status: ToolStatus::Error,
                unavailable: false,
            });
            (
                &call.name,
                call.input.as_ref().map(Value::to_string),
                &r.text,
                r.is_error,
            )
                .hash(&mut h);
            all_unavailable &= r.unavailable;
            blocks.push(Block::ToolResult {
                tool_use_id: call.id.clone(),
                content: Some(r.content),
                is_error: Some(r.is_error),
            });
        }
        BatchOutcome {
            blocks,
            fingerprint: h.finish(),
            all_unavailable,
        }
    }

    async fn flush(
        &self,
        task: &TaskRun,
        agent: &AgentCtx,
        turn: &str,
        group: &mut Vec<Pending>,
        slots: &mut [Option<Res>],
    ) {
        let items = std::mem::take(group);
        let futs = items.iter().map(|(i, id, name, input, spec)| async move {
            (
                *i,
                self.exec_call(task, agent, turn, id, name, input.clone(), spec)
                    .await,
            )
        });
        for (i, r) in join_all(futs).await {
            slots[i] = Some(r);
        }
    }

    /// Counts the call against `max_tool_calls` / the wall clock; `Some` = not allowed to run.
    fn charge(&self, task: &TaskRun, agent: &AgentCtx, turn: &str, call: &Call) -> Option<Res> {
        let mut c = lock(&task.counters);
        let wall = task.started.elapsed().as_millis() as u64;
        let over = if c.tool_calls >= task.budget.max_tool_calls {
            Some((
                "max_tool_calls",
                f64::from(task.budget.max_tool_calls),
                f64::from(c.tool_calls),
            ))
        } else if task.budget.max_wall_ms.is_some_and(|m| wall >= m) {
            Some((
                "max_wall_ms",
                task.budget.max_wall_ms.unwrap_or(0) as f64,
                wall as f64,
            ))
        } else {
            c.tool_calls += 1;
            None
        };
        drop(c);
        let (kind, limit, used) = over?;
        self.mark_budget(task, agent, kind, limit, used);
        Some(self.not_run(
            task,
            agent,
            turn,
            &call.id,
            &call.name,
            ToolStatus::BudgetExceeded,
            format!(
                "Budget exceeded ({kind} = {limit}): the tool was not run and the task is stopping."
            ),
            false,
        ))
    }

    async fn plan_call(&self, task: &TaskRun, agent: &AgentCtx, turn: &str, call: &Call) -> Plan {
        let err = |text: String| {
            Plan::Done(self.not_run(
                task,
                agent,
                turn,
                &call.id,
                &call.name,
                ToolStatus::Error,
                text,
                false,
            ))
        };
        match &call.problem {
            Some(Problem::Truncated) => {
                return err("Tool input was cut off (output truncated at the max_tokens limit or invalid JSON). Retry with a smaller step.".into());
            }
            Some(Problem::Malformed(e)) => {
                return err(format!(
                    "Tool input was not valid JSON ({e}). Send the call again with a JSON object as input."
                ));
            }
            None => {}
        }
        let Some(mut input) = call.input.clone() else {
            return err("Tool input is missing.".into());
        };
        let Some(spec) = self.agent_spec(agent, &call.name) else {
            return err(format!("Unknown tool \"{}\"", call.name));
        };
        if let Err(e) = validate(&spec, &input) {
            // Schema messages quote the offending value.
            return err(if self.tools.is_sensitive(&call.name, &input) {
                format!(
                    "{}: invalid input (details withheld: sensitive input)",
                    call.name
                )
            } else {
                e
            });
        }
        // The interface tools only describe a question; they are not policy-checked here.
        if spec.runtime == ToolRuntime::Harness && call.name != "Task" {
            return Plan::Special { input };
        }
        let denied = |d: Denial| {
            let shown = if d.from_hook {
                "Blocked by a PreToolUse hook (reason withheld from the event log).".to_string()
            } else {
                d.text.clone()
            };
            let mut r = self.not_run(
                task,
                agent,
                turn,
                &call.id,
                &call.name,
                ToolStatus::Denied,
                shown,
                d.unavailable,
            );
            r.content = Value::String(d.text.clone());
            r.text = d.text;
            Plan::Done(r)
        };

        // Policy, then PreToolUse; whenever the input changes after the hook saw it (an approver
        // edit), the hook runs again and the policy rechecks.
        let mut forced: Option<String> = None;
        let mut settled = false;
        for round in 0..4 {
            let before = input.clone();
            if let Err(d) = self
                .authorize(
                    task,
                    agent,
                    turn,
                    call,
                    &spec,
                    &mut input,
                    forced.take(),
                    round > 0,
                )
                .await
            {
                return denied(d);
            }
            if round > 0 && input == before {
                settled = true;
                break;
            }
            let Some(pre) = self
                .run_hook(
                    task,
                    agent,
                    Some(turn),
                    HookEvent::PreToolUse,
                    json!({"tool_name": call.name, "tool_input": self.tools.redact_input(&call.name, &input)}),
                    Some(&call.name),
                )
                .await
            else {
                settled = true;
                break;
            };
            let why = |d: &str| pre.reason.clone().unwrap_or_else(|| d.to_string());
            if pre.blocked || pre.hook_decision == Some(HookDecision::Deny) {
                return denied(Denial {
                    text: format!("Blocked by a PreToolUse hook: {}", why("denied by hook")),
                    unavailable: false,
                    from_hook: true,
                });
            }
            // A hook may tighten (ask) but `allow` from a hook is advisory and ignored.
            forced = (pre.hook_decision == Some(HookDecision::Ask))
                .then(|| "a PreToolUse hook requested confirmation".to_string());
            let rewritten = pre.input_rewritten;
            if let Some(new) = pre.updated_input.clone().filter(|_| rewritten) {
                // The hook only saw the redacted input; its rewrite would carry the placeholder.
                if self.tools.is_sensitive(&call.name, &input) {
                    return err(
                        "A PreToolUse hook cannot rewrite a call with sensitive input.".into(),
                    );
                }
                if let Err(e) = validate(&spec, &new) {
                    return err(format!(
                        "A PreToolUse hook rewrote the input into an invalid one. {e}"
                    ));
                }
                input = new;
            }
            if !rewritten && forced.is_none() {
                settled = true;
                break;
            }
        }
        if !settled {
            return denied(Denial {
                text: "Permission denied: the input kept changing during approval.".into(),
                unavailable: false,
                from_hook: false,
            });
        }
        if spec.runtime == ToolRuntime::Harness {
            return Plan::Special { input };
        }
        if let Some(r) = self.charge(task, agent, turn, call) {
            return Plan::Done(r);
        }
        Plan::Run { input, spec }
    }

    /// Policy decision plus approval. `Err` = the call must not run. Input edited by the
    /// approver is validated and re-checked from scratch, bounded to a few rounds.
    #[allow(clippy::too_many_arguments)]
    async fn authorize(
        &self,
        task: &TaskRun,
        agent: &AgentCtx,
        turn: &str,
        call: &Call,
        spec: &ToolSpec,
        input: &mut Value,
        mut forced: Option<String>,
        recheck: bool,
    ) -> Result<(), Denial> {
        for round in 0..4 {
            let mode = *lock(&agent.mode);
            let mut verdict = {
                let p = lock(&agent.perms);
                if recheck || round > 0 {
                    p.recheck(&call.name, input, mode)
                } else {
                    p.check(&call.name, input, mode)
                }
            };
            if verdict.decision == Decision::Classify {
                // No classifier in the harness: unclassified means a person decides.
                verdict.decision = Decision::Ask;
                verdict.reason = format!("{} (no auto-mode classifier available)", verdict.reason);
            }
            if verdict.decision == Decision::Allow
                && let Some(why) = forced.take()
            {
                verdict.decision = Decision::Ask;
                verdict.reason = why;
            }
            match verdict.decision {
                Decision::Allow => return Ok(()),
                Decision::Deny => {
                    return Err(Denial {
                        unavailable: verdict.reason.starts_with("non-interactive"),
                        from_hook: false,
                        text: verdict.message.map_or_else(
                            || {
                                format!(
                                    "Permission denied: {}. The tool was NOT run. Do not retry the same call; choose another approach or report what is blocked.",
                                    verdict.reason
                                )
                            },
                            String::from,
                        ),
                    });
                }
                _ => {}
            }
            if !lock(&agent.perms).env.interactive {
                return Err(Denial {
                    unavailable: true,
                    from_hook: false,
                    text: format!(
                        "Permission denied: non-interactive run, no one to approve ({}). The tool was NOT run. Choose an approach that needs no approval or report what is blocked.",
                        verdict.reason
                    ),
                });
            }
            let a = self.approve(task, agent, turn, call, input, &verdict).await;
            if a.decision != ApprovalDecision::Allow {
                return Err(Denial {
                    unavailable: a.decision == ApprovalDecision::AutoDenied,
                    from_hook: false,
                    text: a.message.unwrap_or_else(|| DENY_MESSAGE.to_string()),
                });
            }
            self.apply_accepted(agent, &call.name, input, &a);
            match a.updated_input {
                Some(new) if new != *input => {
                    // The approver was shown the redacted input; an edit would carry the placeholder.
                    if self.tools.is_sensitive(&call.name, input) {
                        return Err(Denial {
                            text: "Input of a call with sensitive values cannot be edited during approval."
                                .into(),
                            unavailable: false,
                            from_hook: false,
                        });
                    }
                    validate(spec, &new).map_err(|e| Denial {
                        text: e,
                        unavailable: false,
                        from_hook: false,
                    })?;
                    *input = new;
                    forced = None;
                }
                _ => return Ok(()),
            }
        }
        Err(Denial {
            text: "Permission denied: the input kept changing during approval.".into(),
            unavailable: false,
            from_hook: false,
        })
    }

    /// Applies "allow always" options, but only ones that were offered for this call.
    fn apply_accepted(&self, agent: &AgentCtx, tool: &str, input: &Value, a: &Approval) {
        if a.accepted.is_empty() {
            return;
        }
        let mut p = lock(&agent.perms);
        // Re-derive the offer: an approver cannot smuggle in rules or modes.
        let offered = p.suggestions(tool, input);
        let ok: Vec<Suggestion> = a
            .accepted
            .iter()
            .filter(|s| offered.contains(s))
            .cloned()
            .collect();
        if let Some(m) = p.apply(&ok) {
            *lock(&agent.mode) = m;
        }
    }

    async fn approve(
        &self,
        task: &TaskRun,
        agent: &AgentCtx,
        turn: &str,
        call: &Call,
        input: &Value,
        verdict: &Verdict,
    ) -> Approval {
        let suggestions = lock(&agent.perms).suggestions(&call.name, input);
        let approval_id = uuid();
        let dialog = if verdict.outside_read.is_some() {
            "outside_read"
        } else {
            "permission"
        };
        let shown = self.tools.redact_input(&call.name, input);
        let reason = sanitize_for_display(&verdict.reason);
        let prompt = format!(
            "{} wants to run {}: {}",
            sanitize_for_display(&call.name),
            summarize(&shown),
            reason
        );
        let mut options = vec!["allow".to_string(), "deny".to_string()];
        if !suggestions.is_empty() {
            options.insert(1, "allow_always".into());
        }
        self.emit(
            task,
            agent,
            Some(turn),
            Event::ToolApprovalRequired {
                approval_id: approval_id.clone(),
                tool_call_id: call.id.clone(),
                dialog: dialog.into(),
                prompt: prompt.clone(),
                options,
                suggestions: suggestions.iter().map(suggestion_json).collect(),
                deadline: None,
            },
        );
        self.set_status(
            task,
            nemo_core::SessionStatus::WaitingApproval,
            "approval required",
        );
        let req = ApprovalRequest {
            approval_id: approval_id.clone(),
            agent_id: agent.agent_id.clone(),
            tool_call_id: call.id.clone(),
            tool: call.name.clone(),
            input: shown,
            dialog,
            prompt,
            reason,
            suggestions,
        };
        let a = tokio::select! {
            biased;
            () = task.cancel.cancelled() => Approval {
                reviewer: "policy",
                ..Approval::deny("Cancelled before the approval was answered.")
            },
            a = self.approver.approve(req, &task.cancel) => a,
        };
        self.emit(
            task,
            agent,
            Some(turn),
            Event::ToolApprovalResolved {
                approval_id,
                decision: a.decision.as_str().into(),
                reviewer: a.reviewer.into(),
                input_changed: a.updated_input.is_some(),
                message: a.message.clone(),
            },
        );
        if !task.cancel.is_cancelled() {
            self.set_status(task, nemo_core::SessionStatus::Running, "approval resolved");
        }
        a
    }

    fn render(&self, name: &str, c: &ToolContent) -> (Value, String, bool) {
        let max = self.config.max_tool_result_chars;
        match c {
            ToolContent::Text(t) => {
                let (capped, cut) = cap_text(t, max);
                (
                    Value::String(wrap_tool_output(name, &capped)),
                    t.clone(),
                    cut,
                )
            }
            ToolContent::Blocks(bs) => {
                let (mut vals, mut text, mut cut_any) = (vec![], String::new(), false);
                for b in bs {
                    match b {
                        ToolBlock::Text { text: t } => {
                            let (capped, cut) = cap_text(t, max);
                            vals.push(
                                json!({"type": "text", "text": wrap_tool_output(name, &capped)}),
                            );
                            text.push_str(t);
                            cut_any |= cut;
                        }
                        other => vals.push(serde_json::to_value(other).unwrap_or(Value::Null)),
                    }
                }
                (Value::Array(vals), text, cut_any)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn exec_call(
        &self,
        task: &TaskRun,
        agent: &AgentCtx,
        turn: &str,
        id: &str,
        name: &str,
        input: Value,
        spec: &ToolSpec,
    ) -> Res {
        let ctx = self.tool_ctx(task, agent, turn, id);
        self.emit(
            task,
            agent,
            Some(turn),
            Event::ToolStarted {
                tool_call_id: id.into(),
                name: name.into(),
                side_effect: side_effect_str(spec.side_effect).into(),
                cwd: task.workspace.cwd.to_string_lossy().into_owned(),
            },
        );
        let t0 = Instant::now();
        let deadline = task.deadline;
        let out = tokio::select! {
            biased;
            () = task.cancel.cancelled() => Err(ToolStatus::Interrupted),
            () = async {
                match deadline {
                    Some(d) => tokio::time::sleep_until(tokio::time::Instant::from_std(d)).await,
                    None => std::future::pending().await,
                }
            } => Err(ToolStatus::Timeout),
            o = self.tools.execute(name, input.clone(), &ctx) => Ok(o),
        };
        let out = match out {
            Ok(o) => o,
            Err(status) => {
                // The dropped tool future may have started work of its own: stop it through the
                // call's token. (A child of the task token: the task token itself is still needed
                // for the final verification.) Services a tool started on purpose, such as
                // background terminals or the browser, belong to the session and are closed by
                // the host with `Registry::close_session`, not by this timeout.
                ctx.cancel.cancel();
                let text = if status == ToolStatus::Interrupted {
                    "Interrupted.".to_string()
                } else {
                    "Tool exceeded the task time budget (max_wall_ms) and was stopped.".to_string()
                };
                let res = Res {
                    content: Value::String(text.clone()),
                    text,
                    is_error: true,
                    status,
                    unavailable: false,
                };
                return self.complete(task, agent, turn, id, name, Some(t0), res, false, false);
            }
        };

        // Execute-class tools may have changed anything even when they fail.
        match spec.side_effect {
            SideEffectClass::Write if !out.is_error => {
                task.side_effects.store(true, Ordering::SeqCst);
                task.pending_user_need.store(false, Ordering::SeqCst);
                self.record_changed(task, &input);
            }
            SideEffectClass::Execute => {
                task.side_effects.store(true, Ordering::SeqCst);
                task.pending_user_need.store(false, Ordering::SeqCst);
                // A shell may have changed anything, even when it fails.
                for f in self.tracker.changed(&task.workspace, &task.task_id).await {
                    self.add_changed(task, f);
                }
            }
            _ => {}
        }
        if name == "verify_run" && !out.is_error {
            self.register_model_checks(task, &out.meta);
        }
        if let Some(todos) = out.meta.get("todos").and_then(Value::as_array) {
            let s = |t: &Value, k: &str| {
                t.get(k)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            };
            let todos = todos
                .iter()
                .map(|t| Todo {
                    id: t.get("id").and_then(Value::as_str).map(String::from),
                    content: s(t, "content"),
                    status: s(t, "status"),
                    active_form: s(t, "activeForm"),
                })
                .collect();
            self.emit(task, agent, Some(turn), Event::TodoUpdated { todos });
        }

        let (mut content, text, cut) = self.render(name, &out.content);
        if let Some(post) = self
            .run_hook(
                task,
                agent,
                Some(turn),
                HookEvent::PostToolUse,
                json!({"tool_name": name, "tool_input": self.tools.redact_input(name, &input), "tool_response": text}),
                Some(name),
            )
            .await
            && !post.additional_context.is_empty()
        {
            let extra = post.additional_context.join("\n");
            match &mut content {
                Value::String(s) => {
                    s.push_str("\n\n");
                    s.push_str(&extra);
                }
                Value::Array(a) => a.push(json!({"type": "text", "text": extra})),
                _ => {}
            }
        }
        let status = crate::agent::tool_status_for(out.is_error, out.soft);
        let res = Res {
            content,
            text,
            is_error: out.is_error,
            status,
            unavailable: false,
        };
        self.complete(task, agent, turn, id, name, Some(t0), res, false, cut)
    }

    fn record_changed(&self, task: &TaskRun, input: &Value) {
        let Some(p) = ["file_path", "notebook_path"]
            .iter()
            .find_map(|k| input.get(*k).and_then(Value::as_str))
        else {
            return;
        };
        let ws = &task.workspace;
        let shown = ws
            .resolve(p)
            .ok()
            .and_then(|abs| {
                abs.strip_prefix(&ws.root)
                    .ok()
                    .map(|r| r.to_string_lossy().into_owned())
            })
            .unwrap_or_else(|| p.to_string());
        self.add_changed(task, shown);
    }

    fn add_changed(&self, task: &TaskRun, file: String) {
        let mut c = lock(&task.changed);
        if !c.contains(&file) {
            c.push(file);
        }
    }

    /// Checks the model declared through `verify_run` (`meta.checks`, `declared_by: model`) go
    /// to the completion gate, which re-runs them itself. Same command = same check.
    fn register_model_checks(&self, task: &TaskRun, meta: &Value) {
        let mut all = lock(&task.model_checks);
        for c in meta
            .get("checks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|c| c.get("declared_by").and_then(Value::as_str) == Some("model"))
        {
            let Some(cmd) = c.get("command").and_then(Value::as_str) else {
                continue;
            };
            if all.iter().any(|k| k.command.as_deref() == Some(cmd)) {
                continue;
            }
            let id = c.get("id").and_then(Value::as_str).unwrap_or("m");
            let id = if all.iter().any(|k| k.id == id) {
                format!("m{}", all.len() + 1)
            } else {
                id.to_string()
            };
            all.push(Check {
                id,
                kind: CheckKind::Custom,
                command: Some(cmd.to_string()),
                cwd: task.workspace.cwd.clone(),
                timeout_ms: DEFAULT_TIMEOUT_MS,
                declared_by: Declarer::Model,
                criteria_ids: vec![],
            });
        }
    }

    // ---- harness-run tools ---------------------------------------------------------------

    async fn run_special(
        &self,
        task: &TaskRun,
        agent: &AgentCtx,
        turn: &str,
        call: &Call,
        input: Value,
    ) -> Res {
        if let Some(r) = self.charge(task, agent, turn, call) {
            return r;
        }
        self.emit(
            task,
            agent,
            Some(turn),
            Event::ToolStarted {
                tool_call_id: call.id.clone(),
                name: call.name.clone(),
                side_effect: "read".into(),
                cwd: task.workspace.cwd.to_string_lossy().into_owned(),
            },
        );
        let t0 = Instant::now();
        let res = if call.name == "Task" {
            self.run_subagent(task, agent, turn, &call.id, input).await
        } else {
            self.run_interface(task, agent, turn, call, input).await
        };
        self.complete(
            task,
            agent,
            turn,
            &call.id,
            &call.name,
            Some(t0),
            res,
            false,
            false,
        )
    }

    fn plain(&self, text: impl Into<String>, is_error: bool, status: ToolStatus) -> Res {
        let text = text.into();
        Res {
            content: Value::String(text.clone()),
            text,
            is_error,
            status,
            unavailable: false,
        }
    }

    /// AskUserQuestion / ExitPlanMode: the tool only describes what is needed (`needs_user`);
    /// the answer comes from the `InputProvider`.
    async fn run_interface(
        &self,
        task: &TaskRun,
        agent: &AgentCtx,
        turn: &str,
        call: &Call,
        input: Value,
    ) -> Res {
        let ctx = self.tool_ctx(task, agent, turn, &call.id);
        let out = tokio::select! {
            biased;
            () = task.cancel.cancelled() => return self.plain("Interrupted.", true, ToolStatus::Interrupted),
            o = self.tools.execute(&call.name, input, &ctx) => o,
        };
        let need = out
            .meta
            .get("needs_user")
            .and_then(|v| serde_json::from_value::<NeedsUser>(v.clone()).ok());
        let Some(need) = need.filter(|_| !out.is_error) else {
            let (content, text, _) = self.render(&call.name, &out.content);
            return Res {
                content,
                text,
                is_error: out.is_error,
                status: crate::agent::tool_status_for(out.is_error, out.soft),
                unavailable: false,
            };
        };

        let (dialog, prompt, options) = match &need {
            NeedsUser::AskUserQuestion { questions } => (
                "question",
                questions
                    .first()
                    .map(|q| q.question.clone())
                    .unwrap_or_default(),
                questions
                    .first()
                    .map(|q| q.options.iter().map(|o| o.label.clone()).collect())
                    .unwrap_or_default(),
            ),
            NeedsUser::ExitPlanMode { plan } => (
                "plan",
                preview(plan),
                vec!["approve".to_string(), "keep_planning".to_string()],
            ),
        };
        let approval_id = uuid();
        self.emit(
            task,
            agent,
            Some(turn),
            Event::ToolApprovalRequired {
                approval_id: approval_id.clone(),
                tool_call_id: call.id.clone(),
                dialog: dialog.into(),
                prompt,
                options,
                suggestions: vec![],
                deadline: None,
            },
        );
        self.set_status(
            task,
            nemo_core::SessionStatus::WaitingApproval,
            "user input required",
        );
        let reply = tokio::select! {
            biased;
            () = task.cancel.cancelled() => None,
            r = self.input.ask(&need, &task.cancel) => r,
        };
        let resolved =
            |decision: &str, reviewer: &str, message: Option<String>| Event::ToolApprovalResolved {
                approval_id: approval_id.clone(),
                decision: decision.into(),
                reviewer: reviewer.into(),
                input_changed: false,
                message,
            };
        if task.cancel.is_cancelled() {
            self.emit(
                task,
                agent,
                Some(turn),
                resolved("deny", "user", Some("cancelled".into())),
            );
            return self.plain("Interrupted.", true, ToolStatus::Interrupted);
        }
        self.set_status(
            task,
            nemo_core::SessionStatus::Running,
            "user input resolved",
        );
        match (reply, &need) {
            (None, _) => {
                self.emit(
                    task,
                    agent,
                    Some(turn),
                    resolved("auto_denied", "headless", Some("no user available".into())),
                );
                task.pending_user_need.store(true, Ordering::SeqCst);
                self.plain(
                    "No user is available to answer in this run (headless). Do not ask again: make a reasonable assumption, or stop and state exactly what is blocked.",
                    true,
                    ToolStatus::SoftError,
                )
            }
            (Some(UserReply::Answers(a)), NeedsUser::AskUserQuestion { .. }) => {
                self.emit(task, agent, Some(turn), resolved("allow", "user", None));
                task.pending_user_need.store(false, Ordering::SeqCst);
                self.plain(json!({"answers": a}).to_string(), false, ToolStatus::Ok)
            }
            (
                Some(UserReply::Plan {
                    approved: true,
                    mode,
                    ..
                }),
                NeedsUser::ExitPlanMode { .. },
            ) => {
                self.emit(task, agent, Some(turn), resolved("allow", "user", None));
                let cur = *lock(&agent.mode);
                if let Some(m) = mode.or((cur == Mode::Plan).then_some(Mode::Default)) {
                    *lock(&agent.mode) = m;
                    lock(&agent.perms).set_mode(m);
                }
                self.plain("approved", false, ToolStatus::Ok)
            }
            (
                Some(UserReply::Plan {
                    approved: false,
                    message,
                    ..
                }),
                NeedsUser::ExitPlanMode { .. },
            ) => {
                self.emit(
                    task,
                    agent,
                    Some(turn),
                    resolved("deny", "user", message.clone()),
                );
                self.plain(
                    message.unwrap_or_else(|| "kept planning".into()),
                    true,
                    ToolStatus::SoftError,
                )
            }
            (Some(_), _) => {
                self.emit(
                    task,
                    agent,
                    Some(turn),
                    resolved("deny", "user", Some("mismatched reply".into())),
                );
                self.plain(
                    "The reply did not match the request.",
                    true,
                    ToolStatus::SoftError,
                )
            }
        }
    }

    /// Runs a child agent with its own history. Boxed: the loop calls back into this function.
    fn run_subagent<'a>(
        &'a self,
        task: &'a TaskRun,
        parent: &'a AgentCtx,
        turn: &'a str,
        call_id: &'a str,
        input: Value,
    ) -> crate::traits::BoxFut<'a, Res> {
        Box::pin(async move {
            const DEPTH_LIMIT: u32 = 2;
            if parent.depth >= DEPTH_LIMIT {
                return self.plain(
                    "Task: subagent nesting depth limit reached.",
                    true,
                    ToolStatus::Error,
                );
            }
            let kind = input
                .get("subagent_type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let Some(def) = self.agents.get(kind) else {
                return self.plain(
                    format!("Task: unknown subagent type \"{kind}\"."),
                    true,
                    ToolStatus::Error,
                );
            };
            let prompt = input
                .get("prompt")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();

            // Never wider than the parent: stricter mode, same rules (copied), subset of tools.
            let parent_mode = *lock(&parent.mode);
            let mode = stricter(parent_mode, def.mode.unwrap_or(parent_mode));
            let perms = {
                let mut p = lock(&parent.perms).clone();
                p.set_mode(mode);
                Arc::new(Mutex::new(p))
            };
            let mut allowed: BTreeSet<String> = match &def.tools {
                Some(t) => t.iter().cloned().collect(),
                None => self.tools.specs().into_iter().map(|s| s.name).collect(),
            };
            // Never delegable, even when the definition lists them: no nesting, no questions to
            // the user from a child.
            allowed.retain(|n| !matches!(n.as_str(), "Task" | "AskUserQuestion" | "ExitPlanMode"));
            if let Some(pa) = &parent.allowed {
                allowed.retain(|n| pa.contains(n));
            }
            let child = AgentCtx {
                agent_id: Some(uuid()),
                depth: parent.depth + 1,
                mode: Mutex::new(mode),
                perms,
                allowed: Some(allowed),
                prompt: Some(def.prompt.clone()),
            };
            let info = |reason: Option<String>| nemo_events::SubagentInfo {
                agent_id: child.agent_id.clone().unwrap_or_default(),
                agent_type: def.name.clone(),
                tool_call_id: Some(call_id.to_string()),
                reason,
            };
            self.emit(task, &child, Some(turn), Event::SubagentStarted(info(None)));

            let mut st = AgentState {
                messages: vec![Message {
                    role: Role::User,
                    content: Content::Text(prompt),
                }],
                recent: Default::default(),
                compacted: false,
            };
            let end = self.run_agent(task, &child, &mut st).await;
            let (text, is_error, reason, status) = match end {
                AgentEnd::Finished(t) if t.trim().is_empty() => (
                    "(subagent finished with no text output)".to_string(),
                    false,
                    "finished",
                    ToolStatus::Ok,
                ),
                AgentEnd::Finished(t) => (t, false, "finished", ToolStatus::Ok),
                AgentEnd::Cancelled => (
                    "Interrupted.".into(),
                    true,
                    "cancelled",
                    ToolStatus::Interrupted,
                ),
                AgentEnd::Budget(k) => (
                    format!("Task stopped: budget exceeded ({k})."),
                    true,
                    "budget_exceeded",
                    ToolStatus::BudgetExceeded,
                ),
                AgentEnd::NoProgress { .. } => (
                    "Task stopped: no progress.".into(),
                    true,
                    "no_progress",
                    ToolStatus::Error,
                ),
                AgentEnd::ModelError { reason, .. } => (
                    format!("Task failed: {reason}"),
                    true,
                    "model_error",
                    ToolStatus::Error,
                ),
                AgentEnd::UnsafeWorkspace(w) => (
                    format!("Task stopped: unsafe workspace: {w}"),
                    true,
                    "unsafe_workspace",
                    ToolStatus::Error,
                ),
                AgentEnd::Refusal => (
                    "Task failed: the model refused.".into(),
                    true,
                    "refusal",
                    ToolStatus::Error,
                ),
                AgentEnd::Internal(w) => (
                    format!("Task failed: {w}"),
                    true,
                    "internal",
                    ToolStatus::Error,
                ),
            };
            self.emit(
                task,
                &child,
                Some(turn),
                Event::SubagentStopped(info(Some(reason.into()))),
            );
            self.run_hook(
                task,
                &child,
                Some(turn),
                HookEvent::SubagentStop,
                json!({"agent_type": def.name, "agent_id": child.agent_id, "stop_reason": reason}),
                Some(&def.name),
            )
            .await;

            let content = if is_error {
                Value::String(text.clone())
            } else {
                Value::String(wrap_tool_output("Task", &text))
            };
            Res {
                content,
                text,
                is_error,
                status,
                unavailable: false,
            }
        })
    }
}
