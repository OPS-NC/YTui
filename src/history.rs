//! Persistent search history.
//!
//! One query per line, oldest first, in a plain text file — the format a shell
//! history uses, and for the same reason: it survives being read by a human,
//! and appending costs one write.

use std::path::PathBuf;

const MAX_ENTRIES: usize = 200;

/// ~/.local/share/ytui/search_history (XDG_DATA_HOME honoured).
fn history_path() -> Option<PathBuf> {
    let root = match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        Some(base) => PathBuf::from(base),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".local/share"),
    };
    Some(root.join("ytui").join("search_history"))
}

/// The list plus a cursor into it, as a shell prompt sees its history.
///
/// Cursor semantics: `entries.len()` means "at the live line, nothing recalled".
pub struct SearchHistory {
    path: Option<PathBuf>,
    entries: Vec<String>,
    cursor: usize,
    draft: String,
}

impl SearchHistory {
    pub fn load() -> Self {
        let path = history_path();
        let mut entries: Vec<String> = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map(|text| {
                text.lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();
        let excess = entries.len().saturating_sub(MAX_ENTRIES);
        entries.drain(..excess);
        let cursor = entries.len();
        Self { path, entries, cursor, draft: String::new() }
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        // A read-only home must not take the app down.
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut text = self.entries.join("\n");
        text.push('\n');
        let _ = std::fs::write(path, text);
    }

    pub fn add(&mut self, query: &str) {
        let query = query.trim();
        if query.is_empty() {
            return;
        }
        // A repeated query moves to the end rather than piling up.
        self.entries.retain(|e| e != query);
        self.entries.push(query.to_string());
        let excess = self.entries.len().saturating_sub(MAX_ENTRIES);
        self.entries.drain(..excess);
        self.reset();
        self.save();
    }

    pub fn reset(&mut self) {
        self.cursor = self.entries.len();
        self.draft.clear();
    }

    /// Older entry, or None at the top of the list.
    pub fn previous(&mut self, current: &str) -> Option<String> {
        if self.cursor == self.entries.len() {
            self.draft = current.to_string(); // keep what was being typed
        }
        if self.cursor == 0 {
            return None;
        }
        self.cursor -= 1;
        Some(self.entries[self.cursor].clone())
    }

    /// Newer entry; past the newest, hands the unsent draft back.
    pub fn next(&mut self) -> Option<String> {
        if self.cursor >= self.entries.len() {
            return None;
        }
        self.cursor += 1;
        if self.cursor == self.entries.len() {
            return Some(self.draft.clone());
        }
        Some(self.entries[self.cursor].clone())
    }
}
