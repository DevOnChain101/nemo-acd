use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Envelope. `type` + `payload` come from the flattened [`Event`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    pub v: u16,
    pub seq: u64,
    pub ts: String,
    pub session_id: String,
    pub task_id: Option<String>,
    pub turn_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(flatten)]
    pub event: Event,
}

/// Same envelope with an untyped `type`/`payload`; used for unknown event types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawEnvelope {
    pub v: u16,
    pub seq: u64,
    pub ts: String,
    pub session_id: String,
    pub task_id: Option<String>,
    pub turn_id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub artifact_id: String,
    /// Relative to the session artifact root.
    pub path: String,
    pub size: u64,
    pub sha256: String,
    pub mime: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Budget {
    pub max_turns: u32,
    pub max_tool_calls: u32,
    pub max_cost_usd: Option<f64>,
    pub max_wall_ms: Option<u64>,
    pub max_verify_attempts: u32,
}

impl Default for Budget {
    fn default() -> Self {
        Self {
            max_turns: 50,
            max_tool_calls: 200,
            max_cost_usd: None,
            max_wall_ms: None,
            max_verify_attempts: 3,
        }
    }
}

/// Open enum: unknown values deserialize as `Other` (spec §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Ok,
    Error,
    SoftError,
    Denied,
    Interrupted,
    Timeout,
    BudgetExceeded,
    #[serde(other)]
    Other,
}

/// Task terminal status (`task.ended.status`). Open enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Completed,
    Unverified,
    Blocked,
    Failed,
    Cancelled,
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub workspace_id: String,
    pub root: String,
    pub cwd: String,
    pub additional_dirs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_worktree: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Criterion {
    pub id: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<String>,
}

/// Payload of `session.completed|failed|cancelled`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionEnd {
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Payload of `process.spawned|ready|exited`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProcessInfo {
    pub process_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<String>,
}

/// Payload of `subagent.started|stopped`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubagentInfo {
    pub agent_id: String,
    pub agent_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Todo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub content: String,
    pub status: String,
    pub active_form: String,
}

/// Event taxonomy, adjacently tagged: `{"type": "...", "payload": {...}}`.
/// Fields documented as enums are `String` unless they are `ToolStatus`/`TaskStatus`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum Event {
    #[serde(rename = "session.started")]
    SessionStarted {
        workspace: WorkspaceInfo,
        model: String,
        permission_mode: String,
        source: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        resumed_from_seq: Option<u64>,
    },
    #[serde(rename = "session.status_changed")]
    SessionStatusChanged {
        from: String,
        to: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    #[serde(rename = "session.completed")]
    SessionCompleted(SessionEnd),
    #[serde(rename = "session.failed")]
    SessionFailed(SessionEnd),
    #[serde(rename = "session.cancelled")]
    SessionCancelled(SessionEnd),
    #[serde(rename = "task.started")]
    TaskStarted {
        request: String,
        acceptance_criteria: Vec<Criterion>,
        budget: Budget,
    },
    #[serde(rename = "task.ended")]
    TaskEnded {
        status: TaskStatus,
        reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        verification_report_id: Option<String>,
        usage: Usage,
        changed_files: Vec<String>,
        artifacts_root: String,
    },
    #[serde(rename = "user.message")]
    UserMessage {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        display_text: Option<String>,
        images: Vec<ArtifactRef>,
    },
    #[serde(rename = "turn.started")]
    TurnStarted {
        model: String,
        provider: String,
        iteration: i64,
    },
    #[serde(rename = "model.delta")]
    ModelDelta {
        block_index: i64,
        kind: String,
        text: String,
    },
    #[serde(rename = "assistant.message")]
    AssistantMessage {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
        content: Vec<Value>,
        stop_reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
    },
    #[serde(rename = "turn.completed")]
    TurnCompleted {
        stop_reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
        duration_ms: u64,
    },
    #[serde(rename = "model.retry")]
    ModelRetry {
        attempt: i64,
        max: i64,
        error: String,
        delay_ms: u64,
    },
    #[serde(rename = "model.error")]
    ModelError {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<i64>,
        message: String,
        retryable: bool,
        context_overflow: bool,
    },
    #[serde(rename = "context.compacted")]
    ContextCompacted {
        trigger: String,
        summary_ref: ArtifactRef,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tokens_before: Option<u64>,
    },
    #[serde(rename = "tool.requested")]
    ToolRequested {
        tool_call_id: String,
        name: String,
        input: Value,
        input_truncated: bool,
    },
    #[serde(rename = "tool.approval_required")]
    ToolApprovalRequired {
        approval_id: String,
        tool_call_id: String,
        dialog: String,
        prompt: String,
        options: Vec<String>,
        suggestions: Vec<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        deadline: Option<String>,
    },
    #[serde(rename = "tool.approval_resolved")]
    ToolApprovalResolved {
        approval_id: String,
        decision: String,
        reviewer: String,
        input_changed: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
    },
    #[serde(rename = "tool.started")]
    ToolStarted {
        tool_call_id: String,
        name: String,
        side_effect: String,
        cwd: String,
    },
    #[serde(rename = "tool.completed")]
    ToolCompleted {
        tool_call_id: String,
        name: String,
        status: ToolStatus,
        is_error: bool,
        not_run: bool,
        duration_ms: u64,
        preview: String,
        bytes: u64,
        truncated: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result_ref: Option<ArtifactRef>,
    },
    #[serde(rename = "hook.ran")]
    HookRan {
        event: String,
        hook_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exit_code: Option<i64>,
        outcome: String,
        duration_ms: u64,
    },
    #[serde(rename = "process.spawned")]
    ProcessSpawned(ProcessInfo),
    #[serde(rename = "process.ready")]
    ProcessReady(ProcessInfo),
    #[serde(rename = "process.exited")]
    ProcessExited(ProcessInfo),
    #[serde(rename = "artifact.created")]
    ArtifactCreated {
        #[serde(flatten)]
        artifact: ArtifactRef,
        kind: String,
        retention: String,
    },
    #[serde(rename = "verification.completed")]
    VerificationCompleted {
        report_id: String,
        status: String,
        attempt: i64,
        summary: String,
    },
    #[serde(rename = "todo.updated")]
    TodoUpdated { todos: Vec<Todo> },
    #[serde(rename = "subagent.started")]
    SubagentStarted(SubagentInfo),
    #[serde(rename = "subagent.stopped")]
    SubagentStopped(SubagentInfo),
    #[serde(rename = "model.changed")]
    ModelChanged {
        model: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        effort: Option<String>,
    },
    #[serde(rename = "budget.exceeded")]
    BudgetExceeded {
        budget: String,
        limit: f64,
        used: f64,
    },
    #[serde(rename = "notice")]
    Notice { level: String, text: String },
}
