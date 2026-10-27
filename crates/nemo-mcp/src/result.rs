//! `tools/call` result normalization (port of mcp.js `_convertResult`, :520-561).

use serde_json::Value;

const MAX_TEXT_LEN: usize = 30_000;
const MAX_RESOURCE_TEXT_LEN: usize = 2_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpContent {
    Text(String),
    Image { media_type: String, data: String },
}

/// Normalized tool result. Text-only results are a single `Text` (possibly empty); results with
/// images are the text (if non-empty) followed by the images.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpToolResult {
    pub content: Vec<McpContent>,
    pub is_error: bool,
}

impl McpToolResult {
    pub fn error(msg: impl Into<String>) -> Self {
        Self {
            content: vec![McpContent::Text(msg.into())],
            is_error: true,
        }
    }

    /// Concatenated text blocks.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|c| match c {
                McpContent::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Keeps at most `max` UTF-16 units (JS `length` semantics), then appends the marker.
pub fn truncate(text: &str, max: usize) -> String {
    if text.encode_utf16().count() <= max {
        return text.to_string();
    }
    let mut units = 0;
    let mut out = String::new();
    for c in text.chars() {
        units += c.len_utf16();
        if units > max {
            break;
        }
        out.push(c);
    }
    out.push_str("\n… [truncated]");
    out
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

/// `result` is the JSON form of a `CallToolResult` (`content[]`, `structuredContent`, `isError`).
pub fn normalize_result(result: &Value) -> McpToolResult {
    let mut parts: Vec<String> = Vec::new();
    let mut images = Vec::new();
    for block in result
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match s(block, "type") {
            "text" => parts.push(s(block, "text").into()),
            "image" => images.push(McpContent::Image {
                media_type: Some(s(block, "mimeType"))
                    .filter(|m| !m.is_empty())
                    .unwrap_or("image/png")
                    .into(),
                data: s(block, "data").into(),
            }),
            "audio" => parts.push(format!("[audio {} content omitted]", s(block, "mimeType"))),
            "resource" => {
                let res = block.get("resource").unwrap_or(&Value::Null);
                match res.get("text").and_then(Value::as_str) {
                    Some(t) => parts.push(truncate(t, MAX_RESOURCE_TEXT_LEN)),
                    None => parts.push(format!(
                        "[embedded resource {}{}]",
                        s(res, "uri"),
                        mime_suffix(res)
                    )),
                }
            }
            "resource_link" => parts.push(format!(
                "[resource link {}{}]",
                s(block, "uri"),
                mime_suffix(block)
            )),
            _ => {}
        }
    }
    let mut text = parts.join("\n");
    if text.is_empty()
        && let Some(sc) = result.get("structuredContent")
    {
        text = sc.to_string();
    }
    let text = truncate(&text, MAX_TEXT_LEN);
    let is_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let content = if images.is_empty() {
        vec![McpContent::Text(text)]
    } else {
        let mut c = Vec::new();
        if !text.is_empty() {
            c.push(McpContent::Text(text));
        }
        c.extend(images);
        c
    };
    McpToolResult { content, is_error }
}

fn mime_suffix(v: &Value) -> String {
    match s(v, "mimeType") {
        "" => String::new(),
        m => format!(" ({m})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn text_blocks_join() {
        let r = normalize_result(
            &json!({"content":[{"type":"text","text":"a"},{"type":"text","text":"b"}]}),
        );
        assert_eq!(
            r,
            McpToolResult {
                content: vec![McpContent::Text("a\nb".into())],
                is_error: false
            }
        );
    }

    #[test]
    fn error_flag_and_empty() {
        let r = normalize_result(&json!({"content":[],"isError":true}));
        assert!(r.is_error);
        assert_eq!(r.content, vec![McpContent::Text(String::new())]);
    }

    #[test]
    fn image_with_and_without_text() {
        let img = json!({"type":"image","data":"AAAA"});
        let r = normalize_result(&json!({"content":[img]}));
        assert_eq!(
            r.content,
            vec![McpContent::Image {
                media_type: "image/png".into(),
                data: "AAAA".into()
            }]
        );
        let r = normalize_result(
            &json!({"content":[{"type":"text","text":"hi"},{"type":"image","data":"B","mimeType":"image/jpeg"}]}),
        );
        assert_eq!(r.content.len(), 2);
        assert_eq!(
            r.content[1],
            McpContent::Image {
                media_type: "image/jpeg".into(),
                data: "B".into()
            }
        );
    }

    #[test]
    fn audio_resource_and_link() {
        let r = normalize_result(&json!({"content":[
            {"type":"audio","mimeType":"audio/wav","data":"x"},
            {"type":"resource","resource":{"uri":"file:///a","text":"body"}},
            {"type":"resource","resource":{"uri":"file:///b","mimeType":"image/png","blob":"x"}},
            {"type":"resource_link","uri":"http://l","mimeType":"text/html"},
            {"type":"resource_link","uri":"http://m"}
        ]}));
        assert_eq!(
            r.text(),
            "[audio audio/wav content omitted]\nbody\n[embedded resource file:///b (image/png)]\n[resource link http://l (text/html)]\n[resource link http://m]"
        );
    }

    #[test]
    fn structured_fallback_only_when_no_text() {
        let r = normalize_result(&json!({"content":[],"structuredContent":{"a":1}}));
        assert_eq!(r.text(), r#"{"a":1}"#);
        let r = normalize_result(
            &json!({"content":[{"type":"text","text":"t"}],"structuredContent":{"a":1}}),
        );
        assert_eq!(r.text(), "t");
    }

    #[test]
    fn truncation() {
        let r = normalize_result(&json!({"content":[{"type":"text","text":"x".repeat(30_001)}]}));
        assert_eq!(r.text(), format!("{}\n… [truncated]", "x".repeat(30_000)));
        let r = normalize_result(
            &json!({"content":[{"type":"resource","resource":{"text":"y".repeat(2500)}}]}),
        );
        assert_eq!(r.text(), format!("{}\n… [truncated]", "y".repeat(2000)));
        assert_eq!(truncate("abc", 3), "abc");
    }
}
