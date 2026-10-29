//! Folds a [`StreamEvent`] sequence into a finished assistant message, reconstructing streamed
//! tool arguments and validating them as JSON. Nothing is fabricated: a tool call whose
//! arguments are not valid JSON keeps `input: None` and is listed in `invalid_tool_inputs`.

use crate::error::{ErrorKind, ProviderError};
use crate::types::{Block, Delta, StartBlock, StopReason, StreamEvent, Usage};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct InvalidToolInput {
    pub tool_use_id: String,
    pub raw: String,
    pub error: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct AssistantMessage {
    pub id: Option<String>,
    pub model: Option<String>,
    pub content: Vec<Block>,
    pub stop_reason: Option<StopReason>,
    pub usage: Usage,
    pub invalid_tool_inputs: Vec<InvalidToolInput>,
}

enum Partial {
    Done(Block),
    Tool {
        id: Option<String>,
        name: Option<String>,
        json: String,
    },
    Text(String),
    Thinking {
        text: String,
        signature: Option<String>,
    },
    Redacted(String),
}

#[derive(Default)]
pub struct MessageBuilder {
    msg: AssistantMessage,
    blocks: BTreeMap<usize, Partial>,
    invalid: Vec<InvalidToolInput>,
}

fn bad(msg: impl Into<String>) -> ProviderError {
    ProviderError::new(ErrorKind::InvalidResponse, msg)
}

impl MessageBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, ev: &StreamEvent) -> Result<(), ProviderError> {
        match ev {
            StreamEvent::MessageStart { message } => {
                self.msg.id.clone_from(&message.id);
                self.msg.model.clone_from(&message.model);
                self.msg.usage.merge(&message.usage);
            }
            StreamEvent::ContentBlockStart {
                index,
                content_block,
            } => {
                let p = match content_block {
                    StartBlock::Text { text } => Partial::Text(text.clone()),
                    StartBlock::Thinking {
                        thinking,
                        signature,
                    } => Partial::Thinking {
                        text: thinking.clone(),
                        signature: signature.clone(),
                    },
                    StartBlock::RedactedThinking { data } => Partial::Redacted(data.clone()),
                    StartBlock::ToolUse { id, name, .. } => Partial::Tool {
                        id: id.clone(),
                        name: name.clone(),
                        json: String::new(),
                    },
                    StartBlock::Unknown => return Ok(()),
                };
                self.blocks.insert(*index, p);
            }
            StreamEvent::ContentBlockDelta { index, delta } => {
                if matches!(delta, Delta::Unknown) {
                    return Ok(());
                }
                let p = self
                    .blocks
                    .get_mut(index)
                    .ok_or_else(|| bad(format!("delta for unknown content block {index}")))?;
                match (p, delta) {
                    (Partial::Text(t), Delta::TextDelta { text }) => t.push_str(text),
                    (Partial::Thinking { text, .. }, Delta::ThinkingDelta { thinking }) => {
                        text.push_str(thinking)
                    }
                    (
                        Partial::Thinking { signature, .. },
                        Delta::SignatureDelta { signature: s },
                    ) => {
                        signature.get_or_insert_with(String::new).push_str(s);
                    }
                    (Partial::Tool { json, .. }, Delta::InputJsonDelta { partial_json }) => {
                        json.push_str(partial_json)
                    }
                    _ => {
                        return Err(bad(format!(
                            "delta type does not match content block {index}"
                        )));
                    }
                }
            }
            StreamEvent::ContentBlockStop { index } => self.close(*index)?,
            StreamEvent::MessageDelta { delta, usage } => {
                if delta.stop_reason.is_some() {
                    self.msg.stop_reason.clone_from(&delta.stop_reason);
                }
                if let Some(u) = usage {
                    self.msg.usage.merge(u);
                }
            }
            StreamEvent::MessageStop | StreamEvent::Ping | StreamEvent::Unknown => {}
        }
        Ok(())
    }

    fn close(&mut self, index: usize) -> Result<(), ProviderError> {
        let Some(p) = self.blocks.remove(&index) else {
            return Ok(());
        };
        let block = match p {
            Partial::Done(b) => b,
            Partial::Text(text) => Block::Text { text },
            Partial::Thinking { text, signature } => Block::Thinking {
                thinking: text,
                signature,
            },
            Partial::Redacted(data) => Block::RedactedThinking { data },
            Partial::Tool { id, name, json } => {
                let (Some(id), Some(name)) = (id, name) else {
                    return Err(bad("tool_use block without id or name"));
                };
                let parsed = if json.trim().is_empty() {
                    Ok(Value::Object(Default::default())) // no-argument tool
                } else {
                    serde_json::from_str::<Value>(&json)
                        .map_err(|e| e.to_string())
                        .and_then(|v| {
                            if v.is_object() {
                                Ok(v)
                            } else {
                                Err("tool input must be a JSON object".into())
                            }
                        })
                };
                match parsed {
                    Ok(v) => Block::ToolUse {
                        id,
                        name,
                        input: Some(v),
                    },
                    Err(error) => {
                        self.invalid.push(InvalidToolInput {
                            tool_use_id: id.clone(),
                            raw: json,
                            error,
                        });
                        Block::ToolUse {
                            id,
                            name,
                            input: None,
                        }
                    }
                }
            }
        };
        // Keep block order by index: park finished blocks until the end.
        self.blocks.insert(index, Partial::Done(block));
        Ok(())
    }

    /// Closes blocks that never got a stop event (truncated streams) and returns the message.
    pub fn finish(mut self) -> Result<AssistantMessage, ProviderError> {
        let open: Vec<usize> = self.blocks.keys().copied().collect();
        for i in open {
            self.close(i)?;
        }
        self.msg.content = self
            .blocks
            .into_values()
            .filter_map(|p| {
                if let Partial::Done(b) = p {
                    Some(b)
                } else {
                    None
                }
            })
            .collect();
        self.msg.invalid_tool_inputs = self.invalid;
        Ok(self.msg)
    }
}
