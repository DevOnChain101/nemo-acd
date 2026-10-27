//! Node transcript JSONL compatibility (session-store.md A and C): path layout, the reader
//! (port of history.js `itemsFromEntries` / `sessionMeta` / `loadAllTranscriptItems` and
//! parser.js `parseLines`) and the line builder shared by the live mirror and the exporter.

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use time::format_description::well_known::{Iso8601, Rfc3339};

/// history.js MAX_BYTES: UI loads read at most the last 8 MiB.
pub const TAIL_BYTES: u64 = 8 * 1024 * 1024;
/// history.js MAX_ITEMS.
pub const MAX_ITEMS: usize = 400;
/// Longest directory name we create under `projects/` (most filesystems cap a name at 255 bytes).
const MAX_DIR_NAME: usize = 200;

/// claudecfg.js `sanitizePath`: every UTF-16 unit outside `[A-Za-z0-9]` becomes `-`.
/// Deviation: a result over 200 bytes is cut and gets a hash suffix (Node would fail to
/// write such a file at all; the real cwd is in every line, so readers never need the name).
pub fn sanitize_path(p: &str) -> String {
    let mut s = String::with_capacity(p.len());
    for c in p.chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c);
        } else {
            (0..c.len_utf16()).for_each(|_| s.push('-'));
        }
    }
    if s.len() > MAX_DIR_NAME {
        let h = Sha256::digest(p.as_bytes());
        let hex: String = h.iter().take(6).map(|b| format!("{b:02x}")).collect();
        s.truncate(MAX_DIR_NAME);
        s.push('-');
        s.push_str(&hex);
    }
    s
}

/// `<home>/projects/<sanitize(cwd)>/<session_id>.jsonl` (A.1).
pub fn transcript_path(home: &Path, cwd: &str, session_id: &str) -> PathBuf {
    home.join("projects")
        .join(sanitize_path(cwd))
        .join(format!("{session_id}.jsonl"))
}

fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) | Some(Value::Bool(false)) => false,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(_) => true,
    }
}

/// JS `String.prototype.trim` whitespace (U+FEFF in, U+0085 out).
fn is_js_ws(c: char) -> bool {
    (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
}
fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_ws)
}
fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}

/// parser.js `parseLines`: blank and unparsable lines are dropped; any JSON value is kept.
pub fn parse_lines(text: &str) -> Vec<Value> {
    text.split('\n')
        .filter(|l| !js_trim(l).is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn soft_patterns() -> &'static [Regex] {
    static RE: OnceLock<Vec<Regex>> = OnceLock::new();
    RE.get_or_init(|| {
        // `.` in JS excludes \r,   and   as well as \n.
        let dot = r"[^\n\r\u{2028}\u{2029}]";
        [
            r"has not been read in this conversation \(or changed since\)\. Read it first\."
                .to_string(),
            r#"^Edit: "old_string" (was not found in|matches [0-9]+ times in) "#.to_string(),
            r#"^Edit: "old_string" and "new_string" must be different\."#.to_string(),
            r#"^NotebookEdit: (no cell "|"cell_id" is required for )"#.to_string(),
            format!(r"^(Read|Edit|NotebookEdit): cannot access {dot}*ENOENT"),
            format!(r"^(Read|Edit|Write): {dot}* is a directory\."),
            r"^The user (doesn't want to proceed|declined this read|did not answer)".to_string(),
            r"^Reads outside the working directories are blocked".to_string(),
            r"^Interrupted( before this tool ran)?\.".to_string(),
            r"^Auto mode (could not evaluate|is paused)".to_string(),
            r"^Denied by auto mode classifier".to_string(),
            r"^Removing a critical path was denied".to_string(),
        ]
        .iter()
        .map(|p| Regex::new(p).expect("static regex"))
        .collect()
    })
}

/// tools/soft.js `isSoftToolError`.
pub fn is_soft_tool_error(text: &str) -> bool {
    soft_patterns().iter().any(|re| re.is_match(text))
}

fn text_of(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect(),
        _ => String::new(),
    }
}

fn typed_part(text: &str) -> String {
    if !text.starts_with("PONYTAIL MODE ACTIVE") {
        return text.to_string();
    }
    text.rsplit("\n\n").next().unwrap_or("").to_string()
}

fn is_synthetic(text: &str) -> bool {
    let t = js_trim(text);
    t.is_empty()
        || t.starts_with("<system-reminder>")
        || t.starts_with("<local-command")
        || t.starts_with("Caveat:")
        || t.starts_with("<command-message>")
}

fn date_parse_ms(s: &str) -> Option<i64> {
    let dt = time::OffsetDateTime::parse(s, &Rfc3339)
        .or_else(|_| time::OffsetDateTime::parse(s, &Iso8601::DEFAULT))
        .ok()?;
    Some((dt.unix_timestamp_nanos().div_euclid(1_000_000)) as i64)
}

/// `e.timestamp ? Date.parse(e.timestamp) : 0`; an unparsable value is NaN, JSON `null`.
fn ts_of(e: &Value) -> Value {
    match e.get("timestamp") {
        v if !truthy(v) => json!(0),
        Some(Value::String(s)) => date_parse_ms(s).map_or(Value::Null, |ms| json!(ms)),
        _ => Value::Null,
    }
}

fn image_infos(content: Option<&Value>) -> Value {
    let Some(Value::Array(a)) = content else {
        return json!([]);
    };
    let imgs = a
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("image"))
        .enumerate()
        .map(|(i, b)| {
            let src = b.get("source");
            let mut m = Map::new();
            m.insert("name".into(), json!(format!("image {}", i + 1)));
            if let Some(mt) = src.and_then(|s| s.get("media_type")) {
                m.insert("mediaType".into(), mt.clone());
            }
            let bytes = src
                .and_then(|s| s.get("data"))
                .and_then(Value::as_str)
                .filter(|d| !d.is_empty())
                .map_or(0, |d| (utf16_len(d) * 3 + 2) / 4); // Math.round(len*3/4)
            m.insert("bytes".into(), json!(bytes));
            Value::Object(m)
        })
        .collect();
    Value::Array(imgs)
}

fn id_key(b: &Value) -> String {
    b.get("id")
        .and_then(Value::as_str)
        .unwrap_or("undefined")
        .to_string()
}

/// history.js `itemsFromEntries`. `limit`: keep the last N items; `None` is JS `Infinity`;
/// `Some(0)` returns everything (JS `slice(-0)`).
pub fn items_from_entries(entries: &[Value], limit: Option<usize>) -> Vec<Value> {
    static CMD: OnceLock<(Regex, Regex)> = OnceLock::new();
    let (cmd_re, args_re) = CMD.get_or_init(|| {
        (
            Regex::new(r"<command-name>\s*([^<]+?)\s*</command-name>").expect("static regex"),
            Regex::new(r"(?s)<command-args>(.*?)</command-args>").expect("static regex"),
        )
    });
    let mut items: Vec<Map<String, Value>> = Vec::new();
    let mut tools: std::collections::HashMap<String, usize> = Default::default();
    let mut seen = std::collections::HashSet::new();
    let obj = |v: Value| match v {
        Value::Object(m) => m,
        _ => unreachable!("json! object literal"),
    };
    for e in entries {
        if !e.is_object() || truthy(e.get("isSidechain")) {
            continue;
        }
        let ts = ts_of(e);
        let ty = e.get("type").and_then(Value::as_str).unwrap_or("");
        let message = e.get("message");
        if ty == "user" && truthy(message) && !truthy(e.get("isMeta")) {
            let c = message.and_then(|m| m.get("content"));
            let arr = c.and_then(Value::as_array);
            if let Some(a) = arr.filter(|a| {
                a.iter()
                    .any(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
            }) {
                for b in a {
                    if b.get("type").and_then(Value::as_str) != Some("tool_result") {
                        continue;
                    }
                    let key = b
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .unwrap_or("undefined");
                    let Some(&i) = tools.get(key) else { continue };
                    let result = text_of(b.get("content"));
                    let is_err = truthy(b.get("is_error"));
                    let it = &mut items[i];
                    it.insert(
                        "status".into(),
                        json!(if is_err { "error" } else { "done" }),
                    );
                    it.insert("soft".into(), json!(is_err && is_soft_tool_error(&result)));
                    it.insert("result".into(), json!(result));
                    it.insert("end".into(), ts.clone());
                }
                continue;
            }
            let text = typed_part(&text_of(c));
            if let Some(dt) = e.get("displayText").and_then(Value::as_str) {
                let mut m = obj(
                    json!({"kind": "user", "text": js_trim(dt), "images": image_infos(c), "ts": ts}),
                );
                if let Some(c) = c {
                    m.insert("modelContent".into(), c.clone());
                }
                items.push(m);
                continue;
            }
            if let Some(cm) = cmd_re.captures(&text) {
                let args = args_re
                    .captures(&text)
                    .map(|a| js_trim(&a[1]).to_string())
                    .filter(|a| !a.is_empty());
                let t = match args {
                    Some(a) => format!("{} {a}", &cm[1]),
                    None => cm[1].to_string(),
                };
                items.push(obj(json!({"kind": "user", "text": t, "ts": ts})));
                continue;
            }
            if js_trim(&text).starts_with("[Request interrupted") {
                items.push(obj(
                    json!({"kind": "note", "text": "interrupted", "tone": "warn", "ts": ts}),
                ));
                continue;
            }
            if is_synthetic(&text) {
                continue;
            }
            items.push(obj(
                json!({"kind": "user", "text": js_trim(&text), "images": image_infos(c), "ts": ts}),
            ));
            continue;
        }
        if ty == "assistant" && truthy(message) {
            let m = message.expect("truthy");
            let mid = m
                .get("id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty());
            let blocks = m
                .get("content")
                .and_then(Value::as_array)
                .map_or(&[][..], |v| v);
            for (i, b) in blocks.iter().enumerate() {
                let bt = b.get("type").and_then(Value::as_str).unwrap_or("");
                let text = b.get("text").and_then(Value::as_str);
                if let Some(mid) = mid {
                    let bid = b
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty());
                    let len = if bt == "text" {
                        text.map_or(String::new(), |t| utf16_len(t).to_string())
                    } else {
                        String::new()
                    };
                    let key = format!(
                        "{mid}:{bt}:{}:{len}",
                        bid.map_or(i.to_string(), str::to_string)
                    );
                    if !seen.insert(key) {
                        continue;
                    }
                }
                if bt == "text"
                    && text.is_some_and(|t| !js_trim(t).is_empty() && js_trim(t) != "(empty)")
                {
                    let mut it = obj(json!({"kind": "text", "text": text, "ts": ts}));
                    if e.get("synthetic").and_then(Value::as_str) == Some("queue-summary") {
                        it.insert("final".into(), json!(true));
                        it.insert("synthetic".into(), json!(true));
                    }
                    items.push(it);
                } else if bt == "thinking" && truthy(b.get("thinking")) {
                    items.push(obj(json!({"kind": "thinking", "text": b["thinking"], "start": ts, "end": ts, "ts": ts})));
                } else if bt == "tool_use" {
                    let key = id_key(b);
                    if tools.contains_key(&key) {
                        continue;
                    }
                    let input = if truthy(b.get("input")) {
                        b["input"].clone()
                    } else {
                        json!({})
                    };
                    let mut it = obj(
                        json!({"kind": "tool", "input": input, "status": "done", "start": ts, "ts": ts}),
                    );
                    for k in ["id", "name"] {
                        if let Some(v) = b.get(k) {
                            it.insert(k.into(), v.clone());
                        }
                    }
                    tools.insert(key, items.len());
                    items.push(it);
                }
            }
            continue;
        }
        if ty == "system" {
            let sub = e.get("subtype").and_then(Value::as_str);
            let summary = if truthy(e.get("summary")) {
                e["summary"].clone()
            } else {
                json!("")
            };
            if sub == Some("compact_boundary") {
                let trig = if e.get("trigger").and_then(Value::as_str) == Some("auto") {
                    "auto"
                } else {
                    "restored"
                };
                items.push(obj(
                    json!({"kind": "compact", "trigger": trig, "summary": summary, "ts": ts}),
                ));
            }
            if sub == Some("handoff") {
                items.push(obj(json!({"kind": "handoff", "text": summary, "bootstrap": truthy(e.get("bootstrap")), "ts": ts})));
            }
        }
    }
    // A transcript is only restored, never reattached: an unfinished call did not survive.
    for &i in tools.values() {
        let it = &mut items[i];
        if !it.contains_key("result") && !it.contains_key("end") {
            it.insert("status".into(), json!("interrupted"));
        }
    }
    let n = items.len();
    let skip = match limit {
        Some(l) if l > 0 => n.saturating_sub(l),
        _ => 0,
    };
    items.into_iter().skip(skip).map(Value::Object).collect()
}

/// Reads and parses a transcript. `tail`: only the last N bytes, dropping the partial first
/// line (history.js `readTail`). Invalid UTF-8 is replaced, as Node's decoder does.
pub fn read_entries(path: &Path, tail: Option<u64>) -> std::io::Result<Vec<Value>> {
    let mut f = File::open(path)?;
    let size = f.metadata()?.len();
    let mut skip_first = false;
    if let Some(t) = tail.filter(|&t| size > t) {
        f.seek(SeekFrom::Start(size - t))?;
        skip_first = true;
    }
    let mut r = BufReader::with_capacity(1 << 20, f);
    let mut buf = Vec::new();
    let mut out = Vec::new();
    if skip_first {
        // No newline in the window keeps everything (JS `indexOf` -1 + 1 = 0).
        let mut first = Vec::new();
        r.read_until(b'\n', &mut first)?;
        if first.last() != Some(&b'\n') {
            out.extend(parse_lines(&String::from_utf8_lossy(&first)));
        }
    }
    loop {
        buf.clear();
        if r.read_until(b'\n', &mut buf)? == 0 {
            break;
        }
        out.extend(parse_lines(&String::from_utf8_lossy(&buf)));
    }
    Ok(out)
}

/// history.js `loadAllTranscriptItems`: whole file, no item cap; missing file gives `[]`.
pub fn load_all_items(path: &Path) -> Vec<Value> {
    read_entries(path, None).map_or_else(|_| Vec::new(), |e| items_from_entries(&e, None))
}

/// history.js `loadTranscriptItems`: last 8 MiB, last 400 items.
pub fn load_items_tail(path: &Path) -> Vec<Value> {
    read_entries(path, Some(TAIL_BYTES))
        .map_or_else(|_| Vec::new(), |e| items_from_entries(&e, Some(MAX_ITEMS)))
}

/// history.js `promptsOfItems`.
pub fn prompts_of_items(items: &[Value], max: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for it in items {
        if it.get("kind").and_then(Value::as_str) != Some("user") || truthy(it.get("auto")) {
            continue;
        }
        let t = js_trim(it.get("text").and_then(Value::as_str).unwrap_or(""));
        if !t.is_empty() && out.last().map(String::as_str) != Some(t) {
            out.push(t.to_string());
        }
    }
    let skip = if max > 0 {
        out.len().saturating_sub(max)
    } else {
        0
    };
    out.split_off(skip)
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionMeta {
    pub first_prompt: String,
    pub cwd: String,
}

/// history.js `sessionMeta`: first typed prompt and cwd from the first 256 KiB.
pub fn session_meta(path: &Path) -> SessionMeta {
    let mut head = Vec::new();
    match File::open(path).and_then(|f| f.take(256 * 1024).read_to_end(&mut head)) {
        Ok(_) => {}
        Err(_) => return SessionMeta::default(),
    }
    let head = String::from_utf8_lossy(&head);
    let mut meta = SessionMeta::default();
    let (mut have_cwd, mut have_prompt) = (false, false);
    for line in head.split('\n') {
        let Ok(e) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if !have_cwd && let Some(c) = e.get("cwd").and_then(Value::as_str) {
            meta.cwd = c.to_string();
            have_cwd = !c.is_empty();
        }
        if !have_prompt
            && e.get("type").and_then(Value::as_str) == Some("user")
            && truthy(e.get("message"))
            && !truthy(e.get("isMeta"))
            && !truthy(e.get("isSidechain"))
        {
            let c = e["message"].get("content");
            if c.and_then(Value::as_array).is_some_and(|a| {
                a.iter()
                    .any(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
            }) {
                continue;
            }
            let text = match e.get("displayText").and_then(Value::as_str) {
                Some(d) => js_trim(d).to_string(),
                None => js_trim(&typed_part(&text_of(c))).to_string(),
            };
            if !text.is_empty() && !is_synthetic(&text) {
                meta.first_prompt = text
                    .split(is_js_ws)
                    .filter(|w| !w.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                have_prompt = true;
            }
        }
        if have_cwd && have_prompt {
            break;
        }
    }
    meta
}

/// Splits a parsed Node `user`/`assistant`/`system` entry into the stored pieces.
/// `meta` = `{ts?, top:{extra top-level fields}, msg:{extra message fields}}`.
pub(crate) fn split_entry(e: &Value) -> Option<(&'static str, Value, Value)> {
    let role = match e.get("type").and_then(Value::as_str)? {
        "user" => "user",
        "assistant" => "assistant",
        "system" => "system",
        _ => return None,
    };
    let mut top = Map::new();
    for (k, v) in e.as_object()? {
        if !matches!(
            k.as_str(),
            "type" | "message" | "sessionId" | "cwd" | "timestamp"
        ) {
            top.insert(k.clone(), v.clone());
        }
    }
    let mut meta = Map::new();
    if let Some(ts) = e.get("timestamp").and_then(Value::as_str) {
        meta.insert("ts".into(), json!(ts));
    }
    let mut content = Value::Null;
    if role != "system" {
        let m = e.get("message").filter(|m| m.is_object())?;
        let mut msg = Map::new();
        for (k, v) in m.as_object()? {
            match k.as_str() {
                "content" => content = v.clone(),
                "role" => {}
                _ => {
                    msg.insert(k.clone(), v.clone());
                }
            }
        }
        meta.insert("msg".into(), Value::Object(msg));
    }
    meta.insert("top".into(), Value::Object(top));
    Some((role, content, Value::Object(meta)))
}

/// Builds one A.3-shaped line. `now` is used when the row has no recorded timestamp and
/// `stamp_missing` is set (live writes); imports keep a missing timestamp missing.
pub(crate) fn node_line(
    session_id: &str,
    cwd: &str,
    role: &str,
    content: &Value,
    meta: &Value,
    default_ts: Option<&str>,
) -> Value {
    let mut line = Map::new();
    if let Some(ts) = meta.get("ts").and_then(Value::as_str).or(default_ts) {
        line.insert("timestamp".into(), json!(ts));
    }
    line.insert("sessionId".into(), json!(session_id));
    line.insert("cwd".into(), json!(cwd));
    line.insert("type".into(), json!(role));
    if let Some(top) = meta.get("top").and_then(Value::as_object) {
        for (k, v) in top {
            line.insert(k.clone(), v.clone());
        }
    }
    if role != "system" {
        let mut msg = Map::new();
        msg.insert("role".into(), json!(role));
        if let Some(extra) = meta.get("msg").and_then(Value::as_object) {
            for (k, v) in extra {
                msg.insert(k.clone(), v.clone());
            }
        }
        msg.insert("content".into(), content.clone());
        line.insert("message".into(), Value::Object(msg));
    }
    Value::Object(line)
}
