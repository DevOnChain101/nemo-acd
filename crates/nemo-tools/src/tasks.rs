//! TodoWrite / Task* (one shared in-memory list per session, as in Node) and the two
//! interface tools whose answer comes from the user (`AskUserQuestion`, `ExitPlanMode`).
//! None of them does I/O; the harness reads `meta` to emit events or prompt the user.

use nemo_core::ToolContext;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::ToolOutput;
use crate::native::{Shared, lock};
use crate::util::str_field;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Todo {
    pub id: String,
    pub content: String,
    pub status: String,
    #[serde(rename = "activeForm")]
    pub active_form: String,
    pub description: String,
}

fn with_todos(
    sh: &Shared,
    ctx: &ToolContext,
    f: impl FnOnce(&mut Vec<Todo>) -> ToolOutput,
) -> ToolOutput {
    let s = sh.session(ctx);
    let mut todos = lock(&s.todos);
    let mut out = f(&mut todos);
    if !out.is_error {
        out.meta = json!({"todos": *todos});
    }
    out
}

pub(crate) fn todo_write(sh: &Shared, input: &Value, ctx: &ToolContext) -> ToolOutput {
    let items = input["todos"].as_array().map_or(&[][..], Vec::as_slice);
    if items
        .iter()
        .any(|t| t["content"].as_str().is_none_or(|c| c.trim().is_empty()))
    {
        return ToolOutput::error("TodoWrite: every todo needs a non-empty \"content\".");
    }
    with_todos(sh, ctx, |todos| {
        *todos = items
            .iter()
            .enumerate()
            .map(|(i, t)| Todo {
                id: (i + 1).to_string(),
                content: t["content"].as_str().unwrap_or_default().into(),
                status: t["status"].as_str().unwrap_or_default().into(),
                active_form: str_field(t, "activeForm")
                    .or(t["content"].as_str())
                    .unwrap_or_default()
                    .into(),
                description: String::new(),
            })
            .collect();
        ToolOutput::text(
            "Todos have been modified successfully. Ensure that you continue to use the \
             todo list to track your progress. Please proceed with the current tasks if \
             applicable.",
        )
    })
}

pub(crate) fn task_create(sh: &Shared, input: &Value, ctx: &ToolContext) -> ToolOutput {
    let Some(subject) = str_field(input, "subject").filter(|s| !s.trim().is_empty()) else {
        return ToolOutput::error("TaskCreate: \"subject\" must be a non-empty string.");
    };
    with_todos(sh, ctx, |todos| {
        let id = (todos
            .iter()
            .filter_map(|t| t.id.parse::<u64>().ok())
            .max()
            .unwrap_or(0)
            + 1)
        .to_string();
        todos.push(Todo {
            id: id.clone(),
            content: subject.into(),
            status: "pending".into(),
            active_form: str_field(input, "activeForm").unwrap_or(subject).into(),
            description: str_field(input, "description").unwrap_or_default().into(),
        });
        ToolOutput::text(
            json!({"task": {"id": id, "subject": subject, "status": "pending"}}).to_string(),
        )
    })
}

pub(crate) fn task_get(sh: &Shared, input: &Value, ctx: &ToolContext) -> ToolOutput {
    let id = input["taskId"].as_str().unwrap_or_default();
    with_todos(sh, ctx, |todos| {
        match todos.iter().find(|t| t.id == id) {
        Some(t) => ToolOutput::text(
            json!({"task": {"id": t.id, "subject": t.content, "status": t.status, "activeForm": t.active_form, "description": t.description}}).to_string(),
        ),
        None => ToolOutput::error(format!("TaskGet: no task with id \"{id}\".")),
    }
    })
}

pub(crate) fn task_list(sh: &Shared, _input: &Value, ctx: &ToolContext) -> ToolOutput {
    with_todos(sh, ctx, |todos| {
        let tasks: Vec<Value> = todos
            .iter()
            .map(|t| json!({"id": t.id, "subject": t.content, "status": t.status}))
            .collect();
        ToolOutput::text(json!({"tasks": tasks}).to_string())
    })
}

pub(crate) fn task_update(sh: &Shared, input: &Value, ctx: &ToolContext) -> ToolOutput {
    let id = input["taskId"].as_str().unwrap_or_default();
    with_todos(sh, ctx, |todos| {
        let Some(t) = todos.iter_mut().find(|t| t.id == id) else {
            return ToolOutput::error(format!("TaskUpdate: no task with id \"{id}\"."));
        };
        if let Some(v) = str_field(input, "status") {
            t.status = v.into();
        }
        if let Some(v) = str_field(input, "subject") {
            t.content = v.into();
        }
        if let Some(v) = str_field(input, "activeForm") {
            t.active_form = v.into();
        }
        if let Some(v) = input["description"].as_str() {
            t.description = v.into();
        }
        ToolOutput::text(
            json!({"task": {"id": t.id, "subject": t.content, "status": t.status}}).to_string(),
        )
    })
}

/// What the harness must obtain from the user before the turn can continue. Returned in
/// `ToolOutput.meta["needs_user"]`; the tool itself never blocks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NeedsUser {
    /// Show the questions, answer with the chosen option labels.
    AskUserQuestion { questions: Vec<Question> },
    /// Show the markdown plan; on approval leave plan mode and start implementing.
    ExitPlanMode { plan: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Question {
    pub question: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    #[serde(default, rename = "multiSelect")]
    pub multi_select: bool,
    pub options: Vec<QuestionOption>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

fn needs_user(req: &NeedsUser) -> ToolOutput {
    let what = match req {
        NeedsUser::AskUserQuestion { .. } => "AskUserQuestion: waiting for the user's answer.",
        NeedsUser::ExitPlanMode { .. } => "ExitPlanMode: waiting for the user to approve the plan.",
    };
    ToolOutput::text(what).with_meta(json!({"needs_user": req}))
}

pub(crate) fn ask_user_question(_: &Shared, input: &Value, _: &ToolContext) -> ToolOutput {
    match serde_json::from_value::<Vec<Question>>(input["questions"].clone()) {
        Ok(q) if !q.is_empty() && q.iter().all(|q| !q.options.is_empty()) => {
            needs_user(&NeedsUser::AskUserQuestion { questions: q })
        }
        Ok(_) => {
            ToolOutput::error("AskUserQuestion: need at least one question, each with options.")
        }
        Err(e) => ToolOutput::error(format!("AskUserQuestion: invalid questions: {e}")),
    }
}

pub(crate) fn exit_plan_mode(_: &Shared, input: &Value, _: &ToolContext) -> ToolOutput {
    match str_field(input, "plan").filter(|p| !p.trim().is_empty()) {
        Some(plan) => needs_user(&NeedsUser::ExitPlanMode { plan: plan.into() }),
        None => ToolOutput::error("ExitPlanMode: \"plan\" is required."),
    }
}
