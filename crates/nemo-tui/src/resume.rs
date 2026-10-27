//! `--resume <id>` and the session list, over `nemo-session` (read side only: the UI never
//! writes the store; the host reopens the writer for the resumed session).

use std::path::Path;

use nemo_events::{Envelope, Record};
use nemo_providers::{Content, Message, Role};
use nemo_session::{Reader, SessionRecord};

use crate::SavedSession;

/// A saved session ready to continue: its events fill the screen, its messages are the history
/// of the next task.
#[derive(Debug)]
pub struct Resume {
    pub record: SessionRecord,
    pub events: Vec<Envelope>,
    pub history: Vec<Message>,
}

fn saved(r: &SessionRecord) -> SavedSession {
    let ms = nemo_tui_render::model::parse_ts(&r.updated_at).unwrap_or(0);
    SavedSession {
        id: r.id.clone(),
        cwd: r.cwd.clone(),
        title: r.title.clone().unwrap_or_default(),
        updated_at: r.updated_at.clone(),
        mtime: ms,
    }
}

/// Newest first; `cwd` limits it to one folder.
pub fn list_saved(nemo_home: &Path, cwd: Option<&str>) -> Result<Vec<SavedSession>, String> {
    let r = Reader::open(nemo_home).map_err(|e| e.to_string())?;
    Ok(r.list_sessions(cwd)
        .map_err(|e| e.to_string())?
        .iter()
        .map(saved)
        .collect())
}

/// `id` is a full id or a unique prefix. Unreadable events or messages are an error, not a
/// silently shorter conversation.
pub fn load_resume(nemo_home: &Path, id: &str) -> Result<Resume, String> {
    if id.trim().is_empty() {
        return Err("empty session id".into()); // an empty prefix would match everything
    }
    let r = Reader::open(nemo_home).map_err(|e| e.to_string())?;
    let all = r.list_sessions(None).map_err(|e| e.to_string())?;
    let matches: Vec<_> = all
        .iter()
        .filter(|s| s.id == id || s.id.starts_with(id))
        .collect();
    let record = match matches.as_slice() {
        [one] => (*one).clone(),
        [] => return Err(format!("no saved session matches \"{id}\"")),
        many => {
            let exact = many.iter().find(|s| s.id == id);
            match exact {
                Some(s) => (*s).clone(),
                None => {
                    return Err(format!(
                        "\"{id}\" matches {} sessions; use a longer id",
                        many.len()
                    ));
                }
            }
        }
    };
    let mut events = Vec::new();
    for rec in r
        .read_events(&record.id, 0, usize::MAX >> 1)
        .map_err(|e| e.to_string())?
    {
        // events of a newer schema are skipped on purpose (spec: consumers ignore unknown types)
        if let Record::Known(e) = rec {
            events.push(e);
        }
    }
    let mut history = Vec::new();
    for m in r.read_messages(&record.id).map_err(|e| e.to_string())? {
        if m.compacted {
            continue;
        }
        let role = match m.role.as_str() {
            "user" => Role::User,
            "assistant" => Role::Assistant,
            _ => continue,
        };
        let content: Content = serde_json::from_value(m.content)
            .map_err(|e| format!("message {} of {}: {e}", m.idx, record.id))?;
        history.push(Message { role, content });
    }
    Ok(Resume {
        record,
        events,
        history,
    })
}
