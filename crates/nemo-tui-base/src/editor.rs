//! Multi-line prompt editor with readline-style keys and prompt history (port of `editor.js`).
//! Text is stored as `char`s so the cursor never splits a code point.

use crate::text::is_js_space;

pub const HISTORY_MAX: usize = 200;
/// One limit for every path into the prompt, in code points.
pub const EDITOR_MAX_CODE_POINTS: usize = 65_536;

/// Normalize terminal input before checking its size: CR/CRLF become LF, tabs two spaces,
/// control characters are dropped. `None` when more than `max` characters would result, so an
/// oversized change never touches the prompt or cursor.
pub fn normalized_input(text: &str, max: usize) -> Option<Vec<char>> {
    let mut out = Vec::new();
    let source = text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\t', "  ");
    for ch in source.chars() {
        let cp = ch as u32;
        if ch != '\n' && (cp < 32 || (127..=159).contains(&cp)) {
            continue;
        }
        if out.len() >= max {
            return None;
        }
        out.push(ch);
    }
    Some(out)
}

#[derive(Debug, Default)]
pub struct LineEditor {
    pub chars: Vec<char>,
    pub cur: usize,
    pub history: Vec<String>,
    /// `None`: editing a draft, otherwise browsing `history[i]`.
    h_index: Option<usize>,
    draft: String,
}

impl LineEditor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    /// Replace the text (cursor at the end). `false`, nothing changed, when it is too long.
    pub fn set(&mut self, text: &str) -> bool {
        let Some(chars) = normalized_input(text, EDITOR_MAX_CODE_POINTS) else {
            return false;
        };
        self.chars = chars;
        self.cur = self.chars.len();
        true
    }

    pub fn clear(&mut self) {
        self.set("");
        self.h_index = None;
    }

    /// Insert at the cursor. `false`, nothing changed, when the prompt would get too long.
    pub fn insert(&mut self, s: &str) -> bool {
        let Some(add) = normalized_input(s, EDITOR_MAX_CODE_POINTS - self.chars.len()) else {
            return false;
        };
        let n = add.len();
        self.chars.splice(self.cur..self.cur, add);
        self.cur += n;
        self.h_index = None;
        true
    }

    pub fn backspace(&mut self) {
        if self.cur > 0 {
            self.chars.remove(self.cur - 1);
            self.cur -= 1;
        }
    }

    pub fn del(&mut self) {
        if self.cur < self.chars.len() {
            self.chars.remove(self.cur);
        }
    }

    pub fn left(&mut self) {
        self.cur = self.cur.saturating_sub(1);
    }

    pub fn right(&mut self) {
        if self.cur < self.chars.len() {
            self.cur += 1;
        }
    }

    fn line_start(&self) -> usize {
        let mut i = self.cur;
        while i > 0 && self.chars[i - 1] != '\n' {
            i -= 1;
        }
        i
    }

    fn line_end(&self) -> usize {
        let mut i = self.cur;
        while i < self.chars.len() && self.chars[i] != '\n' {
            i += 1;
        }
        i
    }

    pub fn home(&mut self) {
        self.cur = self.line_start();
    }

    pub fn end(&mut self) {
        self.cur = self.line_end();
    }

    pub fn word_left(&mut self) {
        let mut i = self.cur;
        while i > 0 && is_js_space(self.chars[i - 1]) {
            i -= 1;
        }
        while i > 0 && !is_js_space(self.chars[i - 1]) {
            i -= 1;
        }
        self.cur = i;
    }

    pub fn word_right(&mut self) {
        let mut i = self.cur;
        while i < self.chars.len() && is_js_space(self.chars[i]) {
            i += 1;
        }
        while i < self.chars.len() && !is_js_space(self.chars[i]) {
            i += 1;
        }
        self.cur = i;
    }

    pub fn kill_start(&mut self) {
        let i = self.line_start();
        self.chars.drain(i..self.cur);
        self.cur = i;
    }

    pub fn kill_end(&mut self) {
        let end = self.line_end();
        self.chars.drain(self.cur..end);
    }

    pub fn kill_word(&mut self) {
        let end = self.cur;
        self.word_left();
        self.chars.drain(self.cur..end);
    }

    pub fn multiline(&self) -> bool {
        self.chars.contains(&'\n')
    }

    /// Older history entry (the draft is kept for [`Self::down`]).
    pub fn up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let i = self.h_index.unwrap_or_else(|| {
            self.draft = self.text();
            self.history.len()
        });
        self.h_index = Some(i);
        if i > 0 {
            self.h_index = Some(i - 1);
            let entry = self.history[i - 1].clone();
            self.set(&entry);
        }
    }

    pub fn down(&mut self) {
        let Some(i) = self.h_index else { return };
        if i + 1 >= self.history.len() {
            self.h_index = None;
            let draft = self.draft.clone();
            self.set(&draft);
        } else {
            self.h_index = Some(i + 1);
            let entry = self.history[i + 1].clone();
            self.set(&entry);
        }
    }

    /// Returns the prompt, remembers it (blank and repeated prompts excepted) and clears.
    pub fn submit(&mut self) -> String {
        let t = self.text();
        if !t.trim_matches(is_js_space).is_empty() && self.history.last() != Some(&t) {
            self.history.push(t.clone());
            if self.history.len() > HISTORY_MAX {
                self.history.remove(0);
            }
        }
        self.clear();
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Translated from Node test/editor.test.js.
    #[test]
    fn bounds_are_code_points_normalized_and_atomic() {
        let mut e = LineEditor::new();
        assert!(e.insert("a\r\nb\tc\0\x1bd"));
        assert_eq!(e.text(), "a\nb  cd");
        e.cur = 2;
        let before = e.text();
        assert!(!e.insert(&"x".repeat(EDITOR_MAX_CODE_POINTS)));
        assert_eq!(e.text(), before);
        assert_eq!(e.cur, 2, "rejected input preserves the cursor too");
        e.clear();
        assert!(e.insert(&"😀".repeat(EDITOR_MAX_CODE_POINTS)));
        assert_eq!(
            e.chars.len(),
            EDITOR_MAX_CODE_POINTS,
            "astral characters count once"
        );
        assert!(!e.insert("x"));
    }

    #[test]
    fn cursor_movement_kills_history() {
        let mut e = LineEditor::new();
        e.insert("hello world");
        e.word_left();
        e.insert("big ");
        assert_eq!(e.text(), "hello big world");
        e.kill_end();
        assert_eq!(e.text(), "hello big ");
        e.kill_word();
        assert_eq!(e.text(), "hello ");
        assert_eq!(e.submit(), "hello ");
        e.insert("draft");
        e.up();
        assert_eq!(e.text(), "hello ");
        e.down();
        assert_eq!(e.text(), "draft");
        e.clear();
        e.insert("a\nb");
        assert!(e.multiline());
    }

    #[test]
    fn each_editor_has_its_own_history() {
        let (mut a, mut b) = (LineEditor::new(), LineEditor::new());
        a.insert("only in a");
        a.submit();
        assert!(b.history.is_empty());
        b.up();
        assert_eq!(b.text(), "", "nothing to recall in the other tab");
        a.up();
        assert_eq!(a.text(), "only in a");
    }

    #[test]
    fn line_keys_and_history_cap() {
        let mut e = LineEditor::new();
        e.insert("one\ntwo three");
        e.home();
        assert_eq!(e.cur, 4);
        e.kill_end();
        assert_eq!(e.text(), "one\n");
        e.cur = 4;
        e.insert("x");
        e.kill_start();
        assert_eq!(e.text(), "one\n");
        for i in 0..250 {
            e.insert(&format!("p{i}"));
            e.submit();
        }
        assert_eq!(e.history.len(), HISTORY_MAX);
        assert_eq!(e.history[0], "p50");
        assert_eq!(e.submit(), "");
        assert_eq!(
            e.history.len(),
            HISTORY_MAX,
            "blank prompts are not remembered"
        );
    }
}
