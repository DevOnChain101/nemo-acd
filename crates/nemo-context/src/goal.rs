//! /goal judge (port of goal.js): a separate, tool-less call that decides whether the stated goal
//! is met. It only reads a short tail of the history. The goal text itself is durable state
//! (`TaskState::goal`).

use crate::{SummarizeError, Summarizer};
use nemo_providers::{Block, Content, Message, Role};

pub const GOAL_SYSTEM_PROMPT: &str = "You judge whether a stated goal has been met by an AI coding assistant's work so far. You will be given the goal condition and a short tail of the conversation (the assistant's last reply and recent tool calls). Reply with ONLY strict JSON, no prose, no markdown fences: {\"met\": true|false, \"reason\": \"one short sentence\"}.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalVerdict {
    pub met: bool,
    pub reason: String,
}

pub fn build_goal_tail(messages: &[Message], max_tools: usize) -> String {
    let mut last_text = String::new();
    let mut tools = Vec::new();
    for m in messages.iter().filter(|m| m.role == Role::Assistant) {
        if let Content::Blocks(bs) = &m.content {
            for b in bs {
                match b {
                    Block::Text { text } if !text.is_empty() => last_text = text.clone(),
                    Block::ToolUse { name, input, .. } => {
                        let inp: String = input
                            .as_ref()
                            .map(|v| v.to_string())
                            .unwrap_or_default()
                            .chars()
                            .take(80)
                            .collect();
                        tools.push(format!("{name}({inp})"));
                    }
                    _ => {}
                }
            }
        } else if let Content::Text(t) = &m.content {
            last_text = t.clone();
        }
    }
    let recent = &tools[tools.len().saturating_sub(max_tools)..];
    [
        if last_text.is_empty() {
            "(no assistant text yet)".to_string()
        } else {
            format!("Last assistant reply:\n{last_text}")
        },
        if recent.is_empty() {
            "(no tool calls yet)".to_string()
        } else {
            format!("Recent tool calls:\n{}", recent.join("\n"))
        },
    ]
    .join("\n\n")
}

/// Tolerant parse: strips fences, takes the first `{` to the last `}`, defaults to `met: false`.
pub fn parse_goal_verdict(text: &str) -> GoalVerdict {
    let bad = || GoalVerdict {
        met: false,
        reason: "could not parse judge response".into(),
    };
    let stripped = text.replace("```json", "").replace("```", "");
    let s = stripped.trim();
    let (Some(a), Some(b)) = (s.find('{'), s.rfind('}')) else {
        return bad();
    };
    if b < a {
        return bad();
    }
    match serde_json::from_str::<serde_json::Value>(&s[a..=b]) {
        Ok(v) => GoalVerdict {
            met: v.get("met").and_then(|m| m.as_bool()).unwrap_or(false),
            reason: v
                .get("reason")
                .and_then(|r| r.as_str())
                .unwrap_or("")
                .to_string(),
        },
        Err(_) => bad(),
    }
}

/// Never fails: a judge error reads as "not met" with the error as reason.
pub async fn judge_goal(
    summarizer: &impl Summarizer,
    goal: &str,
    messages: &[Message],
) -> GoalVerdict {
    let prompt = format!("Goal condition: {goal}\n\n{}", build_goal_tail(messages, 8));
    match summarizer.summarize(GOAL_SYSTEM_PROMPT, &prompt).await {
        Ok(t) => parse_goal_verdict(&t),
        Err(SummarizeError(e)) => GoalVerdict {
            met: false,
            reason: format!("judge failed: {e}"),
        },
    }
}
