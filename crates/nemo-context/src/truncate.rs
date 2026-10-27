//! Long tool output: keep head and tail inline, the full text as an artifact file.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Truncated {
    pub text: String,
    /// Where the full output was saved; `None` when nothing was cut or saving failed.
    pub artifact: Option<PathBuf>,
    pub omitted_chars: usize,
}

fn sanitize(name: &str) -> String {
    let s: String = name
        .chars()
        .take(80)
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() { "output".into() } else { s }
}

fn save(root: &Path, name: &str, text: &str) -> std::io::Result<PathBuf> {
    let mut dir = fs::DirBuilder::new();
    dir.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut dir, 0o700);
    dir.create(root)?;
    let base = sanitize(name);
    for n in 0..100 {
        let file = root.join(if n == 0 {
            format!("{base}.log")
        } else {
            format!("{base}-{n}.log")
        });
        // create_new: a planted file or symlink is never written through; 0600: tool output
        // may hold secrets (windows: inherits the profile ACL).
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
        match opts.open(&file) {
            Ok(mut f) => {
                f.write_all(text.as_bytes())?;
                return Ok(file);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other(
        "too many artifacts with the same name",
    ))
}

/// Cuts `text` to about `max_chars` (one third head, two thirds tail: logs end with the error)
/// and saves the full text under `artifact_root/<name>.log`. The marker names the artifact so
/// the model can read the rest on demand. A failed save is stated in the marker.
pub fn truncate_tool_output(
    text: &str,
    max_chars: usize,
    artifact_root: &Path,
    name: &str,
) -> Truncated {
    let total = text.chars().count();
    if total <= max_chars {
        return Truncated {
            text: text.to_string(),
            artifact: None,
            omitted_chars: 0,
        };
    }
    let head_n = max_chars / 3;
    let tail_n = max_chars - head_n;
    let head_end = text
        .char_indices()
        .nth(head_n)
        .map_or(text.len(), |(i, _)| i);
    let tail_start = text
        .char_indices()
        .nth(total - tail_n)
        .map_or(text.len(), |(i, _)| i);
    let omitted = total - head_n - tail_n;
    let (artifact, note) = match save(artifact_root, name, text) {
        Ok(p) => {
            let note = format!("full output saved at {}", p.display());
            (Some(p), note)
        }
        Err(e) => (None, format!("full output could not be saved: {e}")),
    };
    Truncated {
        text: format!(
            "{}\n[... {omitted} chars omitted; {note} ...]\n{}",
            &text[..head_end],
            &text[tail_start..]
        ),
        artifact,
        omitted_chars: omitted,
    }
}
