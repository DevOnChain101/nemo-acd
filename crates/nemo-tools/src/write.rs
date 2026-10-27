use nemo_core::ToolContext;
use serde_json::{Value, json};

use crate::ToolOutput;
use crate::util::*;

pub(crate) fn run(input: &Value, ctx: &ToolContext) -> ToolOutput {
    let Some(file_path) = str_field(input, "file_path") else {
        return ToolOutput::error("Write: \"file_path\" is required.");
    };
    let Some(content) = input.get("content").and_then(Value::as_str) else {
        return ToolOutput::error("Write: \"content\" is required.");
    };
    let abs = match resolve(ctx, "Write", file_path) {
        Ok(p) => p,
        Err(m) => return ToolOutput::error(m),
    };

    let root = match open_root(ctx, "Write", &abs) {
        Ok(r) => r,
        Err(o) => return o,
    };
    let existing = root.metadata(&abs).ok();
    if let Some(md) = &existing {
        if md.is_dir() {
            return ToolOutput::soft_error(format!("Write: {} is a directory.", abs.display()));
        }
        if !md.is_file() {
            return ToolOutput::error(format!(
                "Write: {} is not a regular file (pipe, device or socket).",
                abs.display()
            ));
        }
        if !is_fresh(ctx, &abs, mtime(md)) {
            return ToolOutput::soft_error(format!(
                "Write: {} exists and has not been read in this conversation (or changed since). Read it first.",
                abs.display()
            ));
        }
    }

    if let Err(e) = root.write(&abs, content) {
        return ToolOutput::error(format!("Write: cannot write {}: {e}", abs.display()));
    }
    let md = match root.metadata(&abs) {
        Ok(m) => m,
        Err(e) => return ToolOutput::error(format!("Write: cannot access {}: {e}", abs.display())),
    };
    ledger_set(ctx, &abs, mtime(&md));

    let created = existing.is_none();
    ToolOutput::text(format!(
        "{} {} ({} bytes).",
        if created { "Created" } else { "Updated" },
        abs.display(),
        md.len()
    ))
    .with_meta(json!({"created": created, "bytes": md.len()}))
}
