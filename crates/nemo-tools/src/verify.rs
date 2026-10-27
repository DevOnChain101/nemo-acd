//! verify_run: plans checks with `nemo_verification` and runs them in the session sandbox.
//! Advisory: the CompletionGate re-plans and re-runs at the end and alone decides completion.

use nemo_core::ToolContext;
use nemo_verification::{
    Check, CheckKind, DEFAULT_TIMEOUT_MS, Declarer, InferredOnlyPolicy, VerificationReport,
    VerifyRunner, aggregate, plan_checks,
};
use serde_json::{Value, json};

use crate::native::Shared;
use crate::util::{int_field, str_field};
use crate::{BoxFuture, ToolOutput};

const MODEL_CHECK_ID: &str = "m1";

pub(crate) fn run<'a>(
    sh: &'a Shared,
    input: &'a Value,
    ctx: &'a ToolContext,
) -> BoxFuture<'a, ToolOutput> {
    Box::pin(async move {
        sh.session(ctx);
        let cwd = ctx.workspace.cwd.clone();
        let timeout_ms = int_field(input, "timeout_ms").map_or(DEFAULT_TIMEOUT_MS, |t| {
            t.clamp(1, DEFAULT_TIMEOUT_MS as i64) as u64
        });
        let ids = |k: &str| -> Vec<String> {
            input[k]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(Into::into))
                .collect()
        };
        let (check_ids, command) = (ids("check_ids"), str_field(input, "command"));
        let model: Vec<Check> = command
            .map(|c| Check {
                id: MODEL_CHECK_ID.into(),
                kind: CheckKind::Custom,
                command: Some(c.into()),
                cwd: cwd.clone(),
                timeout_ms,
                declared_by: Declarer::Model,
                criteria_ids: ids("criteria_ids"),
            })
            .into_iter()
            .collect();
        let mut plan = plan_checks(&[], &model, &cwd);
        if !check_ids.is_empty() {
            if let Some(bad) = check_ids.iter().find(|i| !plan.iter().any(|c| &c.id == *i)) {
                let have: Vec<_> = plan.iter().map(|c| c.id.as_str()).collect();
                return ToolOutput::error(format!(
                    "verify_run: no planned check \"{bad}\" (planned: {}).",
                    have.join(", ")
                ));
            }
            plan.retain(|c| check_ids.contains(&c.id));
        } else if command.is_some() {
            plan.retain(|c| c.id == MODEL_CHECK_ID);
        }
        for c in &mut plan {
            c.timeout_ms = timeout_ms;
        }
        let runner = VerifyRunner::new(sh.rt.clone(), &ctx.session_id, ctx.workspace.clone());
        let checks = runner.run(&plan, &ctx.cancel).await;
        let (status, limitations) = aggregate(&mut [], &checks, InferredOnlyPolicy::default());
        let report = VerificationReport {
            id: format!("vr-{}-tool", ctx.tool_call_id),
            task_id: ctx.task_id.clone(),
            session_id: ctx.session_id.clone(),
            attempt: 0,
            status,
            criteria: vec![],
            started_at: checks
                .first()
                .map(|c| c.started_at.clone())
                .unwrap_or_default(),
            finished_at: checks
                .last()
                .map(|c| c.finished_at.clone())
                .unwrap_or_default(),
            checks,
            changed_files: vec![],
            limitations,
        };
        let mut text = report.summary();
        for c in &report.checks {
            let first = c.summary.lines().next().unwrap_or("");
            text += &format!(
                "\n- {} {} `{}`: {first}",
                c.id,
                c.status.as_str(),
                c.command.as_deref().unwrap_or("")
            );
            if let Some(a) = c.artifacts.first() {
                text += &format!(" (log: artifact_read \"{}\")", a.artifact_id);
            }
        }
        for l in &report.limitations {
            text += &format!("\n! {l}");
        }
        text += "\nAdvisory: the completion gate runs its own verification when you finish and decides.";
        let checks: Vec<Value> = report
            .checks
            .iter()
            .map(|c| json!({"id": c.id, "status": c.status.as_str(), "command": c.command, "exit_code": c.exit_code, "declared_by": c.declared_by}))
            .collect();
        ToolOutput::text(text).with_meta(
            json!({"report_id": report.id, "status": report.status.as_str(), "checks": checks}),
        )
    })
}
