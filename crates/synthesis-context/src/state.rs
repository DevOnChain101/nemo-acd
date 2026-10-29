//! Durable task state: kept apart from raw history so compaction can never lose it.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskState {
    pub goal: Option<String>,
    pub constraints: Vec<String>,
    pub changed_files: Vec<String>,
    pub outstanding_checks: Vec<String>,
}

fn push_unique(v: &mut Vec<String>, s: &str) {
    if !v.iter().any(|x| x == s) {
        v.push(s.to_string());
    }
}

impl TaskState {
    pub fn add_constraint(&mut self, s: &str) {
        push_unique(&mut self.constraints, s);
    }
    pub fn add_changed_file(&mut self, s: &str) {
        push_unique(&mut self.changed_files, s);
    }
    pub fn add_check(&mut self, s: &str) {
        push_unique(&mut self.outstanding_checks, s);
    }
    pub fn resolve_check(&mut self, s: &str) {
        self.outstanding_checks.retain(|c| c != s);
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Text for the `<task_state>` block; `None` when there is nothing to say.
    pub fn render(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let mut s = String::new();
        if let Some(g) = &self.goal {
            s.push_str(&format!("Goal: {g}\n"));
        }
        for (title, items) in [
            ("Constraints", &self.constraints),
            ("Changed files", &self.changed_files),
            ("Outstanding checks", &self.outstanding_checks),
        ] {
            if !items.is_empty() {
                s.push_str(&format!("{title}:\n"));
                items.iter().for_each(|i| s.push_str(&format!("- {i}\n")));
            }
        }
        Some(s)
    }
}
