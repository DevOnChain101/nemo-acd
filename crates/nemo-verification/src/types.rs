//! Verification types (docs/specs/verification.md §1-§2).

use nemo_events::ArtifactRef;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    Passed,
    Failed,
    Unverified,
    Blocked,
}

impl VerificationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Unverified => "unverified",
            Self::Blocked => "blocked",
        }
    }
    /// Precedence for "worst evidence wins": failed > blocked > unverified > passed.
    pub(crate) fn severity(self) -> u8 {
        match self {
            Self::Failed => 3,
            Self::Blocked => 2,
            Self::Unverified => 1,
            Self::Passed => 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CriterionSource {
    User,
    Inferred,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckKind {
    Syntax,
    Typecheck,
    Lint,
    Unit,
    Integration,
    Build,
    BrowserE2e,
    Http,
    Manual,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Declarer {
    User,
    /// Inferred by the harness from the repo.
    Harness,
    /// Registered by the model through `verify_run`.
    Model,
}

/// An acceptance criterion; `check` is the user-declared command that proves it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Criterion {
    pub id: String,
    pub text: String,
    pub source: CriterionSource,
    pub check: Option<String>,
    pub status: VerificationStatus,
    pub evidence: Vec<String>,
}

impl Criterion {
    pub fn user(id: &str, text: &str, check: Option<&str>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            source: CriterionSource::User,
            check: check.map(Into::into),
            status: VerificationStatus::Unverified,
            evidence: vec![],
        }
    }
}

/// A planned check (not yet run).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Check {
    pub id: String,
    pub kind: CheckKind,
    /// None for browser/http/manual checks, which this engine cannot execute.
    pub command: Option<String>,
    pub cwd: PathBuf,
    pub timeout_ms: u64,
    pub declared_by: Declarer,
    pub criteria_ids: Vec<String>,
}

/// Default per-check timeout (verification.md §6).
pub const DEFAULT_TIMEOUT_MS: u64 = 600_000;

/// Outcome of running one [`Check`] (the spec's `Check` result fields).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckResult {
    pub id: String,
    pub kind: CheckKind,
    pub command: Option<String>,
    pub cwd: PathBuf,
    pub declared_by: Declarer,
    pub started_at: String,
    pub finished_at: String,
    pub duration_ms: u64,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub status: VerificationStatus,
    pub blocked_reason: Option<String>,
    /// At most 2 KiB.
    pub summary: String,
    pub failure_fingerprint: Option<String>,
    pub criteria_ids: Vec<String>,
    pub artifacts: Vec<ArtifactRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerificationReport {
    pub id: String,
    pub task_id: String,
    pub session_id: String,
    pub attempt: u32,
    pub status: VerificationStatus,
    pub criteria: Vec<Criterion>,
    pub checks: Vec<CheckResult>,
    pub changed_files: Vec<String>,
    pub limitations: Vec<String>,
    pub started_at: String,
    pub finished_at: String,
}

impl VerificationReport {
    pub fn summary(&self) -> String {
        let n = |s| self.checks.iter().filter(|c| c.status == s).count();
        format!(
            "{}: {} passed, {} failed, {} blocked, {} unverified check(s); {} limitation(s)",
            self.status.as_str(),
            n(VerificationStatus::Passed),
            n(VerificationStatus::Failed),
            n(VerificationStatus::Blocked),
            n(VerificationStatus::Unverified),
            self.limitations.len()
        )
    }

    /// The `verification.completed` event payload.
    pub fn to_event(&self) -> nemo_events::Event {
        nemo_events::Event::VerificationCompleted {
            report_id: self.id.clone(),
            status: self.status.as_str().into(),
            attempt: i64::from(self.attempt),
            summary: self.summary(),
        }
    }
}

pub(crate) fn now() -> String {
    let fmt = time::macros::format_description!(
        "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
    );
    time::OffsetDateTime::now_utc()
        .format(&fmt)
        .unwrap_or_default()
}
