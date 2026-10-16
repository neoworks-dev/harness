//! Input history per session: in memory, with the entries also kept in the data directory.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;

/// Entries kept (and loaded) per session.
const LIMIT: usize = 500;

/// Walking through earlier inputs with Up and back with Down.
#[derive(Debug, Clone, Default)]
pub struct History {
    entries: Vec<String>,
    /// Index being shown; None when editing a fresh draft.
    pos: Option<usize>,
    draft: String,
}

impl History {
    pub fn from_entries(entries: Vec<String>) -> History {
        History {
            entries,
            pos: None,
            draft: String::new(),
        }
    }

    pub fn entries(&self) -> &[String] {
        &self.entries
    }

    /// Records a sent input (a repeat of the last one is not added twice).
    pub fn push(&mut self, text: &str) {
        self.pos = None;
        self.draft.clear();
        if text.is_empty() || self.entries.last().is_some_and(|last| last == text) {
            return;
        }
        self.entries.push(text.to_string());
        if self.entries.len() > LIMIT {
            self.entries.remove(0);
        }
    }

    /// Up: the previous entry, remembering `current` as the draft on the way in.
    pub fn older(&mut self, current: &str) -> Option<String> {
        if self.entries.is_empty() {
            return None;
        }
        let pos = match self.pos {
            None => {
                self.draft = current.to_string();
                self.entries.len() - 1
            }
            Some(0) => return None,
            Some(p) => p - 1,
        };
        self.pos = Some(pos);
        Some(self.entries[pos].clone())
    }

    /// Down: the next entry, or the draft after the newest.
    pub fn newer(&mut self) -> Option<String> {
        let pos = self.pos?;
        if pos + 1 < self.entries.len() {
            self.pos = Some(pos + 1);
            Some(self.entries[pos + 1].clone())
        } else {
            self.pos = None;
            Some(std::mem::take(&mut self.draft))
        }
    }

    /// Typing (or anything but Up/Down) ends the walk; the shown text is the new draft.
    pub fn reset(&mut self) {
        self.pos = None;
    }

    pub fn browsing(&self) -> bool {
        self.pos.is_some()
    }
}

/// Where histories live on disk: `<dir>/<session>.jsonl`, one JSON string per line.
#[derive(Debug, Clone, Default)]
pub struct HistoryStore {
    dir: Option<PathBuf>,
}

impl HistoryStore {
    pub fn new(dir: Option<PathBuf>) -> HistoryStore {
        HistoryStore { dir }
    }

    /// `<data dir>/term-history`.
    pub fn in_data_dir() -> HistoryStore {
        HistoryStore::new(crate::util::data_dir().map(|d| d.join("term-history")))
    }

    fn file(&self, session: &str) -> Option<PathBuf> {
        let safe = !session.is_empty()
            && session
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        safe.then(|| {
            self.dir
                .as_ref()
                .map(|d| d.join(format!("{session}.jsonl")))
        })?
    }

    pub fn load(&self, session: &str) -> History {
        let Some(text) = self
            .file(session)
            .and_then(|f| std::fs::read_to_string(f).ok())
        else {
            return History::default();
        };
        let mut entries: Vec<String> = text
            .lines()
            .filter_map(|l| serde_json::from_str::<String>(l).ok())
            .collect();
        let excess = entries.len().saturating_sub(LIMIT);
        entries.drain(..excess);
        History::from_entries(entries)
    }

    /// Appends one entry; failures are ignored (history is a convenience).
    pub fn append(&self, session: &str, text: &str) {
        let Some(file) = self.file(session) else {
            return;
        };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(file)
        else {
            return;
        };
        if let Ok(line) = serde_json::to_string(text) {
            let _ = writeln!(f, "{line}");
        }
    }
}

/// Histories of the sessions seen in this run.
#[derive(Debug, Default)]
pub struct Histories {
    store: HistoryStore,
    loaded: HashMap<String, History>,
}

impl Histories {
    pub fn new(store: HistoryStore) -> Histories {
        Histories {
            store,
            loaded: HashMap::new(),
        }
    }

    pub fn of(&mut self, session: &str) -> &mut History {
        self.loaded
            .entry(session.to_string())
            .or_insert_with(|| self.store.load(session))
    }

    pub fn record(&mut self, session: &str, text: &str) {
        let h = self.of(session);
        let added = h.entries().last().is_none_or(|l| l != text);
        h.push(text);
        if added {
            self.store.append(session, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walks_back_and_forth_and_restores_the_draft() {
        let mut h = History::default();
        assert_eq!(h.older("draft"), None);
        h.push("one");
        h.push("two");
        h.push("two");
        assert_eq!(h.entries(), ["one", "two"]);
        assert_eq!(h.older("draft").as_deref(), Some("two"));
        assert_eq!(h.older("two").as_deref(), Some("one"));
        assert_eq!(h.older("one"), None);
        assert_eq!(h.newer().as_deref(), Some("two"));
        assert_eq!(h.newer().as_deref(), Some("draft"));
        assert_eq!(h.newer(), None);
    }

    #[test]
    fn persists_per_session() {
        let dir = std::env::temp_dir().join(format!("suite-term-hist-{}", std::process::id()));
        let store = HistoryStore::new(Some(dir.clone()));
        let mut a = Histories::new(store.clone());
        a.record("s1", "first");
        a.record("s1", "multi\nline");
        a.record("s2", "other");
        let mut b = Histories::new(store);
        assert_eq!(b.of("s1").entries(), ["first", "multi\nline"]);
        assert_eq!(b.of("s2").entries(), ["other"]);
        assert!(b.of("../etc").entries().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }
}
