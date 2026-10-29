//! `ResourceManager`: per-session ports, temp dirs and a leak check over registered runtimes.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::net::TcpListener;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::{Error, Result};

/// Implemented by the harness for synthesis-terminal process groups, browser contexts, etc.
pub trait SessionResources: Send + Sync {
    /// Human-readable ids of things still alive for this session.
    fn live(&self, session_id: &str) -> Vec<String>;
    /// Ask the runtime to stop everything it owns for this session.
    fn shutdown(&self, session_id: &str);
}

#[derive(Default)]
struct State {
    /// port -> owning session
    ports: BTreeMap<u16, String>,
    sessions: BTreeMap<String, Session>,
    cursor: u32,
}

struct Session {
    artifact_root: PathBuf,
    hooks: Vec<Arc<dyn SessionResources>>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct CloseReport {
    pub released_ports: Vec<u16>,
    /// Resources still alive after `shutdown`; empty means nothing leaked.
    pub leaked: Vec<String>,
}

#[derive(Clone)]
pub struct ResourceManager {
    state: Arc<Mutex<State>>,
    range: Range<u16>,
}

/// A reserved port; released on drop (or earlier by `close_session`).
pub struct PortLease {
    pub port: u16,
    session_id: String,
    state: Arc<Mutex<State>>,
}

impl Drop for PortLease {
    fn drop(&mut self) {
        let mut s = lock(&self.state);
        if s.ports.get(&self.port) == Some(&self.session_id) {
            s.ports.remove(&self.port);
        }
    }
}

fn lock(m: &Mutex<State>) -> MutexGuard<'_, State> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl ResourceManager {
    /// `range` is the ephemeral range to hand out, e.g. `49152..65535`.
    pub fn new(range: Range<u16>) -> Self {
        let cursor = u32::from_le_bytes(uuid::Uuid::new_v4().as_bytes()[..4].try_into().unwrap());
        Self {
            state: Arc::new(Mutex::new(State {
                cursor,
                ..State::default()
            })),
            range,
        }
    }

    pub fn open_session(&self, session_id: &str, artifact_root: PathBuf) {
        lock(&self.state).sessions.insert(
            session_id.to_string(),
            Session {
                artifact_root,
                hooks: Vec::new(),
            },
        );
    }

    pub fn register(&self, session_id: &str, hook: Arc<dyn SessionResources>) -> Result<()> {
        lock(&self.state)
            .sessions
            .get_mut(session_id)
            .ok_or_else(|| Error::Invalid(format!("unknown session {session_id}")))?
            .hooks
            .push(hook);
        Ok(())
    }

    /// Reserves a port nobody (this manager or the OS) holds. Bind-tested on 127.0.0.1; the
    /// port can still be taken by an outside process before the caller binds it (TOCTOU).
    pub fn reserve_port(&self, session_id: &str) -> Result<PortLease> {
        let width = u32::from(self.range.end - self.range.start);
        let mut s = lock(&self.state);
        if !s.sessions.contains_key(session_id) {
            return Err(Error::Invalid(format!("unknown session {session_id}")));
        }
        for _ in 0..width {
            s.cursor = s.cursor.wrapping_add(1);
            let port = self.range.start + (s.cursor % width) as u16;
            if s.ports.contains_key(&port) || TcpListener::bind(("127.0.0.1", port)).is_err() {
                continue;
            }
            s.ports.insert(port, session_id.to_string());
            return Ok(PortLease {
                port,
                session_id: session_id.to_string(),
                state: Arc::clone(&self.state),
            });
        }
        Err(Error::Refused("no free port in range".into()))
    }

    /// Fresh directory `<artifact_root>/.workspace-tmp/<label>-<uuid>`, removed on close.
    pub fn temp_dir(&self, session_id: &str, label: &str) -> Result<PathBuf> {
        if label.is_empty()
            || !label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(Error::Invalid(format!("bad temp label {label:?}")));
        }
        let root = lock(&self.state)
            .sessions
            .get(session_id)
            .map(|s| s.artifact_root.clone())
            .ok_or_else(|| Error::Invalid(format!("unknown session {session_id}")))?;
        let dir = root
            .join(".workspace-tmp")
            .join(format!("{label}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// Stops registered runtimes, frees ports, deletes temp dirs, and reports anything that is
    /// still alive. The session stays closed even if something leaked.
    pub fn close_session(&self, session_id: &str) -> Result<CloseReport> {
        let session = lock(&self.state)
            .sessions
            .remove(session_id)
            .ok_or_else(|| Error::Invalid(format!("unknown session {session_id}")))?;
        let mut leaked = Vec::new();
        for h in &session.hooks {
            h.shutdown(session_id);
            leaked.extend(h.live(session_id));
        }
        let mut s = lock(&self.state);
        let released: BTreeSet<u16> = s
            .ports
            .iter()
            .filter(|(_, o)| o.as_str() == session_id)
            .map(|(p, _)| *p)
            .collect();
        for p in &released {
            s.ports.remove(p);
        }
        drop(s);
        match fs::remove_dir_all(session.artifact_root.join(".workspace-tmp")) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => leaked.push(format!("temp dir: {e}")),
        }
        Ok(CloseReport {
            released_ports: released.into_iter().collect(),
            leaked,
        })
    }
}
