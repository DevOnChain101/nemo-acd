//! tool_use / tool_result pairing: every tool_use in an assistant message is answered by a
//! tool_result in the immediately following user message and vice versa.

use nemo_providers::{Block, Content, Message, Role};
use std::collections::HashSet;

pub(crate) fn blocks(m: &Message) -> &[Block] {
    match &m.content {
        Content::Blocks(b) => b,
        Content::Text(_) => &[],
    }
}

fn use_ids(m: &Message) -> HashSet<&str> {
    blocks(m)
        .iter()
        .filter_map(|b| match b {
            Block::ToolUse { id, .. } => Some(id.as_str()),
            _ => None,
        })
        .collect()
}

fn result_ids(m: &Message) -> HashSet<&str> {
    blocks(m)
        .iter()
        .filter_map(|b| match b {
            Block::ToolResult { tool_use_id, .. } => Some(tool_use_id.as_str()),
            _ => None,
        })
        .collect()
}

pub(crate) fn has_tool_result(m: &Message) -> bool {
    blocks(m)
        .iter()
        .any(|b| matches!(b, Block::ToolResult { .. }))
}

/// `Err` names the first violation.
pub fn validate_pairing(msgs: &[Message]) -> Result<(), String> {
    for (i, m) in msgs.iter().enumerate() {
        let uses = use_ids(m);
        let results = result_ids(m);
        if !uses.is_empty() {
            if m.role != Role::Assistant {
                return Err(format!(
                    "message {i}: tool_use outside an assistant message"
                ));
            }
            let next = msgs.get(i + 1).filter(|n| n.role == Role::User);
            let answered = next.map(result_ids).unwrap_or_default();
            if let Some(id) = uses.iter().find(|id| !answered.contains(*id)) {
                return Err(format!("message {i}: tool_use {id} has no result"));
            }
        }
        if has_tool_result(m) {
            if m.role != Role::User {
                return Err(format!("message {i}: tool_result outside a user message"));
            }
            let offered = i
                .checked_sub(1)
                .map(|p| &msgs[p])
                .filter(|p| p.role == Role::Assistant)
                .map(use_ids)
                .unwrap_or_default();
            if let Some(id) = results.iter().find(|id| !offered.contains(*id)) {
                return Err(format!("message {i}: tool_result {id} has no tool_use"));
            }
        }
    }
    Ok(())
}

fn into_blocks(c: Content) -> Vec<Block> {
    match c {
        Content::Blocks(b) => b,
        Content::Text(t) if t.is_empty() => vec![],
        Content::Text(text) => vec![Block::Text { text }],
    }
}

/// Makes any message list pairing-valid: drops orphan tool_results and unanswered tool_uses,
/// removes messages that became empty and merges the adjacent same-role neighbours that result.
/// Adjacent same-role messages are merged into one (a valid alternating list is unchanged).
pub fn repair_pairing(msgs: Vec<Message>) -> Vec<Message> {
    let keep: Vec<(HashSet<String>, HashSet<String>)> = (0..msgs.len())
        .map(|i| {
            let next = msgs
                .get(i + 1)
                .filter(|n| n.role == Role::User && msgs[i].role == Role::Assistant);
            let prev = i
                .checked_sub(1)
                .map(|p| &msgs[p])
                .filter(|p| p.role == Role::Assistant && msgs[i].role == Role::User);
            let ok_uses = next
                .map(result_ids)
                .map(|r| {
                    use_ids(&msgs[i])
                        .intersection(&r)
                        .map(|s| s.to_string())
                        .collect()
                })
                .unwrap_or_default();
            let ok_results = prev
                .map(use_ids)
                .map(|u| {
                    result_ids(&msgs[i])
                        .intersection(&u)
                        .map(|s| s.to_string())
                        .collect()
                })
                .unwrap_or_default();
            (ok_uses, ok_results)
        })
        .collect();
    let mut out: Vec<Message> = Vec::with_capacity(msgs.len());
    for (m, (uses, results)) in msgs.into_iter().zip(keep) {
        let was_blocks = matches!(m.content, Content::Blocks(_));
        let role = m.role;
        let mut bs = into_blocks(m.content);
        let before = bs.len();
        bs.retain(|b| match b {
            Block::ToolUse { id, .. } => uses.contains(id),
            Block::ToolResult { tool_use_id, .. } => results.contains(tool_use_id),
            _ => true,
        });
        if bs.is_empty() && (before > 0 || was_blocks) {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.role == role && role != Role::System => {
                let mut merged = into_blocks(std::mem::replace(
                    &mut last.content,
                    Content::Blocks(vec![]),
                ));
                merged.extend(bs);
                last.content = Content::Blocks(merged);
            }
            _ => out.push(Message {
                role,
                content: if was_blocks {
                    Content::Blocks(bs)
                } else {
                    Content::Text(text_of(bs))
                },
            }),
        }
    }
    out
}

fn text_of(bs: Vec<Block>) -> String {
    bs.into_iter()
        .filter_map(|b| {
            if let Block::Text { text } = b {
                Some(text)
            } else {
                None
            }
        })
        .collect()
}

/// Index where history may be cut, searched from `from` backwards (keeping more) and then
/// forwards: the message at the cut carries no tool_result, so nothing in the tail points back
/// into the dropped head. Index 0 is never returned (that would cut nothing); `len` means no
/// cut exists and everything is head.
pub(crate) fn safe_cut(msgs: &[Message], from: usize) -> usize {
    let ok = |i: usize| !has_tool_result(&msgs[i]);
    (1..=from.min(msgs.len()))
        .rev()
        .chain(from + 1..msgs.len())
        .find(|&i| i < msgs.len() && ok(i))
        .unwrap_or(msgs.len())
}
