//! What was downloaded, installed or updated, kept between runs in `history.json` next to the settings.
//!
//! The download queue forgets finished rows when they are cleared and knows nothing about updates; this list
//! only grows (the newest 500 entries are kept) and is emptied only when the person asks for it.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const MAX: usize = 500;

static LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    /// Unix time in milliseconds.
    pub at: i64,
    /// `install` | `update`
    pub kind: String,
    pub name: String,
    pub file: Option<String>,
    /// `done` | `failed`
    pub state: String,
    pub detail: Option<String>,
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn path(dir: &Path) -> PathBuf {
    dir.join("history.json")
}

/// Oldest first. A missing or unreadable file is an empty history.
pub fn list(dir: &Path) -> Vec<Entry> {
    std::fs::read_to_string(path(dir)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn write(dir: &Path, entries: &[Entry]) {
    let p = path(dir);
    let mut tmp = p.clone().into_os_string();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    if let Ok(text) = serde_json::to_string(entries) {
        if std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, &p).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

pub fn push(dir: &Path, entry: Entry) {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut all = list(dir);
    all.push(entry);
    if all.len() > MAX {
        all.drain(..all.len() - MAX);
    }
    write(dir, &all);
}

pub fn clear(dir: &Path) {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _ = std::fs::remove_file(path(dir));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(n: usize) -> Entry {
        Entry { at: n as i64, kind: "install".into(), name: format!("mod {n}"), file: None, state: "done".into(), detail: None }
    }

    #[test]
    fn entries_stay_between_reads_and_only_the_newest_are_kept() {
        let d = tempfile::tempdir().unwrap();
        assert!(list(d.path()).is_empty());
        for n in 0..MAX + 3 {
            push(d.path(), entry(n));
        }
        let all = list(d.path());
        assert_eq!(all.len(), MAX);
        assert_eq!(all.first().unwrap().name, "mod 3");
        assert_eq!(all.last().unwrap().name, format!("mod {}", MAX + 2));
        clear(d.path());
        assert!(list(d.path()).is_empty());
    }

    #[test]
    fn a_damaged_file_is_an_empty_history_and_is_replaced_by_the_next_entry() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("history.json"), "not json").unwrap();
        assert!(list(d.path()).is_empty());
        push(d.path(), entry(1));
        assert_eq!(list(d.path()).len(), 1);
    }
}
