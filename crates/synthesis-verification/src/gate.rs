//! Status aggregation (verification.md §3), completion gate (§4) and bounded
//! self-correction (§5).

use crate::plan::plan_checks;
use crate::runner::{CheckAuthorizer, VerifyRunner};
use crate::types::*;
use crate::weaken::{FileEdit, removed_prerequisite_guards, weakened_tests};
use std::collections::{BTreeSet, HashMap};
use synthesis_events::TaskStatus;
use tokio_util::sync::CancellationToken;

/// Open question in verification.md §3 rule 4 / doc §13.7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InferredOnlyPolicy {
    /// Inferred checks alone never give `passed`; the report caps at `unverified`.
    CapUnverified,
    /// verification.md §3 rule 4: no failed check and at least one passed -> `passed`.
    #[default]
    AllowPassed,
}

#[derive(Debug, Clone)]
pub struct GateConfig {
    pub max_verify_attempts: u32,
    pub inferred_only: InferredOnlyPolicy,
}

impl Default for GateConfig {
    fn default() -> Self {
        Self {
            max_verify_attempts: synthesis_events::Budget::default().max_verify_attempts,
            inferred_only: InferredOnlyPolicy::default(),
        }
    }
}

/// What the harness knows about the task when the model ends its turn.
#[derive(Debug, Clone, Default)]
pub struct TaskState {
    pub task_id: String,
    pub session_id: String,
    pub criteria: Vec<Criterion>,
    /// Checks the model registered through `verify_run`.
    pub model_checks: Vec<Check>,
    /// Any Write/Execute side effect happened in the task.
    pub side_effects: bool,
    pub changed_files: Vec<String>,
    /// Line diffs of the files the agent edited (for [`weakened_tests`]).
    pub edits: Vec<FileEdit>,
    /// Check command -> skipped + todo tests it already had before the agent's edits. Empty when
    /// no baseline run exists, which makes every skip count against the check.
    pub baseline_skipped: HashMap<String, u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GateDecision {
    /// `None` = the gate did not apply (no side effects, no criteria): `verification_report_id: null`.
    Complete(Option<Box<VerificationReport>>),
    /// Append `feedback` as a user message and go back to `running`.
    Continue {
        feedback: String,
        report: Box<VerificationReport>,
    },
    Stop {
        status: TaskStatus,
        reason: String,
        report: Option<Box<VerificationReport>>,
    },
}

/// Computes criterion statuses in place and returns (report status, limitations).
pub fn aggregate(
    criteria: &mut [Criterion],
    checks: &[CheckResult],
    policy: InferredOnlyPolicy,
) -> (VerificationStatus, Vec<String>) {
    use VerificationStatus::*;
    for c in criteria.iter_mut() {
        let linked: Vec<_> = checks
            .iter()
            .filter(|k| k.criteria_ids.contains(&c.id))
            .collect();
        c.evidence = linked.iter().map(|k| k.id.clone()).collect();
        let worst = linked.iter().map(|k| k.status).max_by_key(|s| s.severity());
        c.status = match worst {
            None => Unverified,
            Some(Failed) => Failed,
            _ if linked.iter().any(|k| k.status == Passed) => Passed,
            Some(w) => w,
        };
    }
    let mut lim = vec![];
    for k in checks
        .iter()
        .filter(|k| k.status != Passed && k.status != Failed)
    {
        if let Some(t) = k.tests.as_ref().filter(|t| t.skipped + t.todo > 0) {
            lim.push(format!(
                "{SKIPPED_TESTS}: check {} ({}) ran {} passed, {} failed but {} skipped and {} todo{}",
                k.id,
                k.command.as_deref().unwrap_or("no command"),
                t.passed,
                t.failed,
                t.skipped,
                t.todo,
                if t.skipped_names.is_empty() {
                    String::new()
                } else {
                    format!(": {}", t.skipped_names.join(" | "))
                }
            ));
            continue;
        }
        lim.push(format!(
            "check {} ({}) was {}: {}",
            k.id,
            k.command.as_deref().unwrap_or("no command"),
            k.status.as_str(),
            k.blocked_reason.as_deref().unwrap_or("it proves nothing")
        ));
    }
    let user: Vec<_> = criteria
        .iter()
        .filter(|c| c.source == CriterionSource::User)
        .collect();
    let status = if checks.iter().any(|k| k.status == Failed) {
        Failed
    } else if user.iter().any(|c| c.status == Blocked) {
        Blocked
    } else if user.iter().any(|c| c.status != Passed) {
        Unverified
    } else if user.is_empty() {
        // `Failed` is excluded above, so one passed check is enough; others become limitations.
        let inferred_pass = checks.iter().any(|k| k.status == Passed);
        if !checks.is_empty() {
            lim.push("no explicit acceptance criteria; inferred checks only".into());
        }
        if inferred_pass && policy == InferredOnlyPolicy::AllowPassed {
            Passed
        } else {
            Unverified
        }
    } else {
        Passed
    };
    if checks.is_empty() {
        lim.push("no checks could be planned or run".into());
    }
    lim.push("a pass covers only what the listed checks exercise, not all behavior".into());
    (status, lim)
}

/// A passed check that skipped tests (beyond its baseline) is not a clean pass.
fn downgrade_skips(checks: &mut [CheckResult], baseline: &HashMap<String, u64>) {
    for k in checks
        .iter_mut()
        .filter(|k| k.status == VerificationStatus::Passed)
    {
        let Some(t) = &k.tests else { continue };
        let was = k.command.as_ref().and_then(|c| baseline.get(c)).copied();
        if t.skipped + t.todo > was.unwrap_or(0) {
            k.status = VerificationStatus::Unverified;
        }
    }
}

/// Per-task gate: create one per task, call `evaluate` each time the model ends its turn.
pub struct CompletionGate {
    runner: VerifyRunner,
    cfg: GateConfig,
    attempt: u32,
    unverified_round_used: bool,
    prev: Option<Prev>,
}

struct Prev {
    fingerprints: BTreeSet<String>,
    changed: Vec<String>,
    by_command: HashMap<String, VerificationStatus>,
}

impl CompletionGate {
    pub fn new(runner: VerifyRunner, cfg: GateConfig) -> Self {
        Self {
            runner,
            cfg,
            attempt: 0,
            unverified_round_used: false,
            prev: None,
        }
    }

    /// `claimed_done == false` means the harness forces a final evaluation (budget exhausted):
    /// it never returns `Continue`, and the status comes from the report alone.
    pub async fn evaluate(
        &mut self,
        st: &TaskState,
        claimed_done: bool,
        cancel: &CancellationToken,
    ) -> GateDecision {
        self.evaluate_with(st, claimed_done, cancel, None).await
    }

    /// [`evaluate`](Self::evaluate) where each check command is first authorised by `auth`; a
    /// refused check is reported `blocked` (never failed, never skipped silently).
    pub async fn evaluate_with(
        &mut self,
        st: &TaskState,
        claimed_done: bool,
        cancel: &CancellationToken,
        auth: Option<&dyn CheckAuthorizer>,
    ) -> GateDecision {
        let has_user = st
            .criteria
            .iter()
            .any(|c| c.source == CriterionSource::User);
        if !has_user && !st.side_effects {
            return GateDecision::Complete(None);
        }
        self.attempt += 1;
        let started_at = now();
        let plan = plan_checks(
            &st.criteria,
            &st.model_checks,
            &self.runner.workspace().cwd,
            &st.changed_files,
        );
        let mut checks = self.runner.run_with(&plan, cancel, auth).await;
        downgrade_skips(&mut checks, &st.baseline_skipped);
        let mut criteria = st.criteria.clone();
        let (mut status, mut limitations) =
            aggregate(&mut criteria, &checks, self.cfg.inferred_only);
        // Editing what the checks assert voids a pass: it is reported, never `passed`.
        for l in [
            weakened_tests(&st.edits),
            removed_prerequisite_guards(&st.edits),
        ]
        .into_iter()
        .flatten()
        {
            limitations.push(l);
            if status == VerificationStatus::Passed {
                status = VerificationStatus::Unverified;
            }
        }

        if let Some(p) = &self.prev {
            for k in checks
                .iter()
                .filter(|k| k.status == VerificationStatus::Failed)
            {
                let was = k.command.as_ref().and_then(|c| p.by_command.get(c));
                if was == Some(&VerificationStatus::Passed) {
                    limitations.push(format!(
                        "regression: check {} ({}) passed in the previous attempt and now fails",
                        k.id,
                        k.command.as_deref().unwrap_or("")
                    ));
                }
            }
        }
        let fingerprints: BTreeSet<String> = checks
            .iter()
            .filter_map(|k| k.failure_fingerprint.clone())
            .collect();
        let no_progress = status == VerificationStatus::Failed
            && self
                .prev
                .as_ref()
                .is_some_and(|p| p.fingerprints == fingerprints && p.changed == st.changed_files);
        self.prev = Some(Prev {
            fingerprints,
            changed: st.changed_files.clone(),
            by_command: checks
                .iter()
                .filter_map(|k| Some((k.command.clone()?, k.status)))
                .collect(),
        });

        let report = Box::new(VerificationReport {
            id: format!("vr-{}-{}", st.task_id, self.attempt),
            task_id: st.task_id.clone(),
            session_id: st.session_id.clone(),
            attempt: self.attempt,
            status,
            criteria,
            checks,
            changed_files: st.changed_files.clone(),
            limitations,
            started_at,
            finished_at: now(),
        });
        let stop = |status, reason: &str, report| GateDecision::Stop {
            status,
            reason: reason.into(),
            report: Some(report),
        };
        if cancel.is_cancelled() {
            return stop(TaskStatus::Cancelled, "cancelled", report);
        }
        let room = claimed_done && self.attempt < self.cfg.max_verify_attempts;
        match status {
            VerificationStatus::Passed => GateDecision::Complete(Some(report)),
            VerificationStatus::Blocked => {
                stop(TaskStatus::Blocked, "verification_blocked", report)
            }
            VerificationStatus::Failed if no_progress && claimed_done => {
                stop(TaskStatus::Failed, "no_progress", report)
            }
            VerificationStatus::Failed if room => {
                let feedback = self.feedback(&report);
                GateDecision::Continue { feedback, report }
            }
            VerificationStatus::Failed => stop(
                TaskStatus::Failed,
                if claimed_done {
                    "verify_budget_exhausted"
                } else {
                    "verification_failed"
                },
                report,
            ),
            VerificationStatus::Unverified if room && !self.unverified_round_used => {
                self.unverified_round_used = true;
                let feedback = self.feedback(&report);
                GateDecision::Continue { feedback, report }
            }
            VerificationStatus::Unverified => stop(TaskStatus::Unverified, "unverified", report),
        }
    }

    /// Structured user-role message for the model; never claims success.
    fn feedback(&self, r: &VerificationReport) -> String {
        let mut m = format!(
            "Verification attempt {} of {} is {}. The task is NOT verified as complete.\n",
            r.attempt,
            self.cfg.max_verify_attempts,
            r.status.as_str()
        );
        for k in r
            .checks
            .iter()
            .filter(|k| k.status == VerificationStatus::Failed)
        {
            m += &format!(
                "\nFAILED {} `{}` ({}):\n",
                k.id,
                k.command.as_deref().unwrap_or(""),
                if k.timed_out {
                    "timeout".into()
                } else {
                    format!("exit {:?}", k.exit_code)
                }
            );
            match k.artifacts.first() {
                Some(a) => {
                    m += &format!(
                        "{}\n(full log: artifact {})\n",
                        self.runner.read_tail(a, 4096),
                        a.artifact_id
                    )
                }
                None => m += &k.summary,
            }
        }
        m += "\nNot verified:\n";
        for c in r
            .criteria
            .iter()
            .filter(|c| c.status != VerificationStatus::Passed)
        {
            m += &format!("- criterion {} ({}): {}\n", c.id, c.text, c.status.as_str());
        }
        for l in &r.limitations {
            m += &format!("- {l}\n");
        }
        m += if r.status == VerificationStatus::Failed {
            "\nFix the cause of the failure, then finish again; checks will be re-run."
        } else {
            "\nAdd or run a focused check that proves each unverified criterion (register it with verify_run), then finish again."
        };
        m
    }
}
