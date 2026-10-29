//! artifact_read: files under `WorkspaceContext.artifact_root` only. Opened through a `cap-std`
//! directory handle, so `..` and symlinks cannot leave the root; absolute paths must already
//! lie inside it. Output is capped and binary files are described, not dumped.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

use cap_std::ambient_authority;
#[cfg(unix)]
use cap_std::fs::OpenOptionsExt;
use cap_std::fs::{Dir, OpenOptions};
use regex::Regex;
use serde_json::{Value, json};
use synthesis_core::{ToolContext, contains, lexical_normalize};

use crate::native::Shared;
use crate::util::{MAX_FILE_BYTES, int_field, is_binary, str_field};
use crate::{BoxFuture, ToolOutput};

const DEFAULT_LIMIT: u64 = 30_000;
const MAX_LIMIT: u64 = 1_000_000;
const MAX_DEPTH: usize = 4;

pub(crate) fn read<'a>(
    _: &'a Shared,
    input: &'a Value,
    ctx: &'a ToolContext,
) -> BoxFuture<'a, ToolOutput> {
    let (input, ctx) = (input.clone(), ctx.clone());
    Box::pin(async move {
        tokio::task::spawn_blocking(move || run(&input, &ctx).unwrap_or_else(ToolOutput::error))
            .await
            .unwrap_or_else(|e| ToolOutput::error(format!("tool task failed: {e}")))
    })
}

/// A bare id is looked up by file name or stem (terminal logs are `<id>-<tag>.log`).
fn find(dir: &Dir, rel: &Path, id: &str, depth: usize) -> Option<PathBuf> {
    let mut subdirs = vec![];
    for e in dir.entries().ok()?.flatten() {
        let name = e.file_name();
        let p = Path::new(&name);
        match e.file_type() {
            Ok(t) if t.is_file() => {
                if name.to_str() == Some(id) || p.file_stem().and_then(|s| s.to_str()) == Some(id) {
                    return Some(rel.join(&name));
                }
            }
            Ok(t) if t.is_dir() => subdirs.push(name),
            _ => {} // symlinks are never followed here
        }
    }
    if depth == 0 {
        return None;
    }
    subdirs
        .into_iter()
        .find_map(|n| find(&dir.open_dir(&n).ok()?, &rel.join(&n), id, depth - 1))
}

/// Path of the artifact relative to the root.
fn locate(root: &Dir, ctx: &ToolContext, id: &str) -> Result<PathBuf, String> {
    let p = Path::new(id);
    let outside = || format!("\"{id}\" is outside the artifact root.");
    if p.is_absolute() {
        let base = synthesis_core::canonicalize(&ctx.workspace.artifact_root)
            .map_err(|_| "this session has no artifacts yet.".to_string())?;
        let abs = lexical_normalize(p);
        if !contains(&base, &abs, ctx.workspace.path_policy) {
            return Err(outside());
        }
        return Ok(abs.components().skip(base.components().count()).collect());
    }
    if !p.components().all(|c| matches!(c, Component::Normal(_))) {
        return Err(outside());
    }
    if p.components().count() > 1 || root.is_file(p) {
        return Ok(p.to_path_buf());
    }
    find(root, Path::new(""), id, MAX_DEPTH).ok_or_else(|| format!("no artifact \"{id}\"."))
}

fn run(input: &Value, ctx: &ToolContext) -> Result<ToolOutput, String> {
    let id =
        str_field(input, "artifact_id").ok_or("artifact_read: \"artifact_id\" is required.")?;
    let e = |m: &str| format!("artifact_read: {m}");
    let re = str_field(input, "filter")
        .map(Regex::new)
        .transpose()
        .map_err(|x| e(&format!("invalid filter regex: {x}")))?;
    let offset = int_field(input, "offset").unwrap_or(0).max(0) as u64;
    let limit =
        int_field(input, "limit").map_or(DEFAULT_LIMIT, |l| (l.max(1) as u64).min(MAX_LIMIT));

    let root = Dir::open_ambient_dir(&ctx.workspace.artifact_root, ambient_authority())
        .map_err(|_| e("this session has no artifacts yet."))?;
    let rel = locate(&root, ctx, id).map_err(|m| e(&m))?;
    let mut o = OpenOptions::new();
    o.read(true);
    #[cfg(unix)]
    o.custom_flags(libc::O_NONBLOCK); // a FIFO must not block the worker
    let mut f = root
        .open_with(&rel, &o)
        .map_err(|x| e(&format!("cannot read \"{id}\": {x}")))?;
    let md = f.metadata().map_err(|x| e(&x.to_string()))?;
    if !md.is_file() {
        return Err(e("not a regular file."));
    }
    let total = md.len();

    // With a filter the whole (bounded) file is needed; otherwise only the requested window.
    let mut buf = vec![];
    if re.is_some() {
        if total > MAX_FILE_BYTES {
            return Err(e(
                "file is too large to filter; page it with offset and limit.",
            ));
        }
        f.read_to_end(&mut buf).map_err(|x| e(&x.to_string()))?;
    } else {
        f.seek(SeekFrom::Start(offset))
            .map_err(|x| e(&x.to_string()))?;
        f.take(limit)
            .read_to_end(&mut buf)
            .map_err(|x| e(&x.to_string()))?;
    }
    let shown = rel.display();
    if is_binary(&buf) {
        return Ok(ToolOutput::text(format!(
            "[binary artifact {shown}: {total} bytes, content not shown]"
        ))
        .with_meta(json!({"size": total, "binary": true})));
    }
    // `len` is the size of what offset/limit page over: the file, or the filtered lines.
    let (window, len) = match re {
        Some(re) => {
            let kept = String::from_utf8_lossy(&buf)
                .lines()
                .filter(|l| re.is_match(l))
                .collect::<Vec<_>>()
                .join("\n")
                .into_bytes();
            let (from, to) = (
                offset.min(kept.len() as u64),
                (offset + limit).min(kept.len() as u64),
            );
            let n = kept.len() as u64;
            (kept[from as usize..to as usize].to_vec(), n)
        }
        None => (buf, total),
    };
    let end = offset.min(len) + window.len() as u64;
    let more = end < len;
    let mut text = String::from_utf8_lossy(&window).into_owned();
    if more {
        text += &format!("\n[{shown}: bytes {offset}-{end} of {len}; continue with offset {end}]");
    }
    Ok(ToolOutput::text(text)
        .with_meta(json!({"size": total, "binary": false, "next_offset": more.then_some(end)})))
}
