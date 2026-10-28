//! `tools/call` result normalization (port of mcp.js `_convertResult`, :520-561), with two
//! deliberate differences: `structuredContent` is kept verbatim next to the text, and a result
//! that is too long loses the MIDDLE of its largest strings (never the tail, where fields such as
//! `version` usually sit) and says so; the full text goes to an artifact file when one is set.

use std::path::Path;

use serde_json::Value;
use sha1::{Digest, Sha1};

/// Characters of text handed to the model. Below the harness cap (30 000) so it never cuts again.
pub const MAX_TEXT_LEN: usize = 28_000;
/// Smallest piece a string is shrunk to before the whole text is cut instead.
const MIN_KEEP: usize = 64;

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
    /// `structuredContent` exactly as the server sent it (never shortened).
    pub structured: Option<Value>,
}

impl McpToolResult {
    pub fn error(msg: impl Into<String>) -> Self {
        Self {
            content: vec![McpContent::Text(msg.into())],
            is_error: true,
            structured: None,
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

/// `keep` chars of `s`: 60 % head, 40 % tail, an explicit marker between.
fn cut_middle(s: &str, keep: usize) -> String {
    let n = s.chars().count();
    if n <= keep {
        return s.to_string();
    }
    let head = keep * 6 / 10;
    let tail = keep - head;
    let h = s.char_indices().nth(head).map_or(s.len(), |(i, _)| i);
    let t = s.char_indices().nth(n - tail).map_or(s.len(), |(i, _)| i);
    format!(
        "{}[… {} chars omitted …]{}",
        &s[..h],
        n - head - tail,
        &s[t..]
    )
}

fn for_each_string(v: &mut Value, f: &mut dyn FnMut(&mut String)) {
    match v {
        Value::String(s) => f(s),
        Value::Array(a) => a.iter_mut().for_each(|x| for_each_string(x, f)),
        Value::Object(o) => o.values_mut().for_each(|x| for_each_string(x, f)),
        _ => {}
    }
}

/// Text of at most `max` chars. A JSON object/array keeps ALL its keys and short values (ids,
/// versions, titles): only its longest strings are shortened in the middle, longest first. Any
/// other text, or JSON that cannot shrink enough this way, is cut in the middle.
pub fn fit(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    if let Ok(mut v @ (Value::Object(_) | Value::Array(_))) = serde_json::from_str::<Value>(text) {
        for _ in 0..64 {
            let out = v.to_string();
            if out.chars().count() <= max {
                return out;
            }
            let mut longest = 0;
            for_each_string(&mut v, &mut |s| longest = longest.max(s.chars().count()));
            if longest <= MIN_KEEP * 2 {
                break;
            }
            let keep = (longest / 2).max(MIN_KEEP);
            let mut done = false;
            for_each_string(&mut v, &mut |s| {
                if !done && s.chars().count() == longest {
                    *s = cut_middle(s, keep);
                    done = true;
                }
            });
        }
    }
    cut_middle(text, max.saturating_sub(40)) // room for the marker
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

/// [`normalize_with`] without an artifact directory.
pub fn normalize_result(result: &Value) -> McpToolResult {
    normalize_with(result, None)
}

/// `result` is the JSON form of a `CallToolResult` (`content[]`, `structuredContent`, `isError`).
/// With `artifacts`, a result longer than [`MAX_TEXT_LEN`] is also written there in full and the
/// text names the file.
pub fn normalize_with(result: &Value, artifacts: Option<&Path>) -> McpToolResult {
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
                    Some(t) => parts.push(t.into()),
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
            other => parts.push(format!("[unsupported content block {other:?} omitted]")),
        }
    }
    let structured = result.get("structuredContent").filter(|v| !v.is_null());
    let mut text = parts.join("\n");
    if text.is_empty()
        && let Some(sc) = structured
    {
        text = sc.to_string();
    }
    let text = if text.chars().count() > MAX_TEXT_LEN {
        match artifacts.and_then(|d| save(d, &text)) {
            // the note is short; leave room for it
            Some(file) => format!(
                "{}\n[full result saved to {file}]",
                fit(&text, MAX_TEXT_LEN - 300)
            ),
            None => fit(&text, MAX_TEXT_LEN),
        }
    } else {
        text
    };
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
    McpToolResult {
        content,
        is_error,
        structured: structured.cloned(),
    }
}

/// Writes `text` to `<dir>/mcp-<sha1 prefix>.txt`; the path on success.
fn save(dir: &Path, text: &str) -> Option<String> {
    let hash: String = Sha1::digest(text.as_bytes())
        .iter()
        .take(6)
        .map(|b| format!("{b:02x}"))
        .collect();
    let file = dir.join(format!("mcp-{hash}.txt"));
    std::fs::create_dir_all(dir).ok()?;
    std::fs::write(&file, text).ok()?;
    Some(file.display().to_string())
}

/// `resources/read` result as a `tools/call`-shaped value (text blocks).
pub fn resource_as_result(v: &Value) -> Value {
    let parts: Vec<Value> = v
        .get("contents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|c| {
            let text = match c.get("text").and_then(Value::as_str) {
                Some(t) => t.to_string(),
                None => format!(
                    "[binary resource {}{} omitted]",
                    s(c, "uri"),
                    mime_suffix(c)
                ),
            };
            serde_json::json!({"type":"text","text":text})
        })
        .collect();
    serde_json::json!({"content": parts})
}

/// `prompts/get` result as a `tools/call`-shaped value: `role: text` per message.
pub fn prompt_as_result(v: &Value) -> Value {
    let mut parts: Vec<Value> = Vec::new();
    if let Some(d) = v.get("description").and_then(Value::as_str) {
        parts.push(serde_json::json!({"type":"text","text":d}));
    }
    for m in v
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let c = m.get("content").unwrap_or(&Value::Null);
        let body = match s(c, "type") {
            "text" => s(c, "text").to_string(),
            "resource" => c["resource"]["text"]
                .as_str()
                .unwrap_or("[resource]")
                .to_string(),
            t => format!("[{t} content omitted]"),
        };
        parts.push(serde_json::json!({"type":"text","text":format!("{}: {body}", s(m, "role"))}));
    }
    serde_json::json!({"content": parts})
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
                is_error: false,
                structured: None
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
    fn structured_is_kept_verbatim_and_is_the_text_only_when_there_is_none() {
        let r = normalize_result(&json!({"content":[],"structuredContent":{"a":1}}));
        assert_eq!(r.text(), r#"{"a":1}"#);
        assert_eq!(r.structured, Some(json!({"a":1})));
        let r = normalize_result(
            &json!({"content":[{"type":"text","text":"t"}],"structuredContent":{"a":1}}),
        );
        assert_eq!(
            (r.text().as_str(), r.structured),
            ("t", Some(json!({"a":1})))
        );
        let big = json!({"b": "z".repeat(MAX_TEXT_LEN * 2)});
        let r = normalize_result(&json!({"content":[],"structuredContent":big.clone()}));
        assert_eq!(
            r.structured,
            Some(big),
            "the structured copy is never shortened"
        );
        assert!(r.text().chars().count() <= MAX_TEXT_LEN);
    }

    #[test]
    fn unknown_blocks_are_named_not_dropped() {
        let r = normalize_result(&json!({"content":[{"type":"hologram"}]}));
        assert!(r.text().contains("hologram"), "{}", r.text());
    }

    #[test]
    fn long_text_loses_its_middle_and_says_so() {
        let t = format!("HEAD{}TAIL", "x".repeat(MAX_TEXT_LEN * 2));
        let r = normalize_result(&json!({"content":[{"type":"text","text":t}]}));
        let out = r.text();
        assert!(out.chars().count() <= MAX_TEXT_LEN, "{}", out.len());
        assert!(out.starts_with("HEAD") && out.ends_with("TAIL"));
        assert!(out.contains("chars omitted"));
        // embedded resource text is no longer cut at 2 000 chars
        let r = normalize_result(
            &json!({"content":[{"type":"resource","resource":{"text":"y".repeat(5000)}}]}),
        );
        assert_eq!(r.text().len(), 5000);
        assert_eq!(truncate("abc", 3), "abc");
        assert_eq!(truncate("abcd", 3), "abc\n… [truncated]");
    }

    #[test]
    fn json_keeps_every_key_and_short_value_when_it_shrinks() {
        let page = json!({"id":"100","title":"T","body":"b".repeat(MAX_TEXT_LEN * 3),
            "labels":["l".repeat(MAX_TEXT_LEN)],"version":{"number":3}});
        let out = fit(&page.to_string(), MAX_TEXT_LEN);
        assert!(out.chars().count() <= MAX_TEXT_LEN);
        let v: Value = serde_json::from_str(&out).expect("still JSON");
        assert_eq!(
            (&v["id"], &v["title"], &v["version"]),
            (&json!("100"), &json!("T"), &json!({"number":3}))
        );
        assert!(v["body"].as_str().unwrap().contains("chars omitted"));
        // not JSON: plain middle cut
        assert!(fit(&"q".repeat(100), 50).contains("chars omitted"));
        assert_eq!(fit("short", 50), "short");
    }

    #[test]
    fn oversized_result_is_saved_in_full_when_a_directory_is_given() {
        let d = tempfile::tempdir().unwrap();
        let t = "w".repeat(MAX_TEXT_LEN + 10);
        let r = normalize_with(
            &json!({"content":[{"type":"text","text":t}]}),
            Some(d.path()),
        );
        let out = r.text();
        assert!(out.chars().count() <= MAX_TEXT_LEN && out.contains("full result saved to"));
        let f = std::fs::read_dir(d.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        assert_eq!(std::fs::read_to_string(f.path()).unwrap(), t);
    }
}
