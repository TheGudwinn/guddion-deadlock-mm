//! Append-only operation journal (JSON lines) with one-call rollback per batch.
//! File based on purpose: rollback must keep working even if the database is unavailable.
use super::error::InstallError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    /// A file we created; rollback deletes it.
    CreateFile { path: PathBuf },
    /// A directory we created; rollback removes it if empty.
    CreateDir { path: PathBuf },
    /// A file we overwrote; rollback copies `backup` back.
    ReplaceFile { path: PathBuf, backup: PathBuf },
    /// A file we deleted (backup kept); rollback copies the backup back.
    DeleteFile { path: PathBuf, backup: PathBuf },
    /// A rename; rollback renames back.
    Rename { from: PathBuf, to: PathBuf },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    pub id: u64,
    pub ts: i64,
    pub batch: String,
    pub op: Op,
    pub rolled_back: bool,
}

pub struct Journal {
    path: PathBuf,
}

fn jerr(e: impl std::fmt::Display) -> InstallError {
    InstallError::Journal(e.to_string())
}

impl Journal {
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn entries(&self) -> Result<Vec<Entry>, InstallError> {
        let text = match std::fs::read_to_string(&self.path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
            Err(e) => return Err(e.into()),
        };
        text.lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).map_err(jerr)).collect()
    }

    fn write_all(&self, entries: &[Entry]) -> Result<(), InstallError> {
        let mut s = String::new();
        for e in entries {
            s.push_str(&serde_json::to_string(e).map_err(jerr)?);
            s.push('\n');
        }
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, s)?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    /// Record an operation. Call this BEFORE performing a destructive step where possible.
    pub fn record(&self, batch: &str, op: Op) -> Result<Entry, InstallError> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut all = self.entries()?;
        let entry = Entry {
            id: all.last().map(|e| e.id + 1).unwrap_or(1),
            ts: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0),
            batch: batch.to_string(),
            op,
            rolled_back: false,
        };
        all.push(entry.clone());
        self.write_all(&all)?;
        Ok(entry)
    }

    /// Undo every not-yet-rolled-back entry of `batch`, newest first. Returns how many were undone.
    pub fn rollback_batch(&self, batch: &str) -> Result<usize, InstallError> {
        let mut all = self.entries()?;
        let mut n = 0;
        for i in (0..all.len()).rev() {
            if all[i].batch != batch || all[i].rolled_back {
                continue;
            }
            undo(&all[i].op)?;
            all[i].rolled_back = true;
            n += 1;
            self.write_all(&all)?; // persist progress so a crash mid-rollback is resumable
        }
        Ok(n)
    }
}

fn undo(op: &Op) -> Result<(), InstallError> {
    match op {
        Op::CreateFile { path } => match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        },
        Op::CreateDir { path } => {
            let _ = std::fs::remove_dir(path); // only succeeds when empty
            Ok(())
        }
        Op::ReplaceFile { path, backup } | Op::DeleteFile { path, backup } => {
            std::fs::copy(backup, path)?;
            Ok(())
        }
        Op::Rename { from, to } => {
            if to.exists() {
                std::fs::rename(to, from)?;
            }
            Ok(())
        }
    }
}

pub fn ensure_within(root: &Path, p: &Path) -> Result<(), InstallError> {
    // Lexical check (files may not exist yet): no `..` components and must start with root.
    if p.components().any(|c| matches!(c, std::path::Component::ParentDir)) || !p.starts_with(root) {
        return Err(InstallError::OutsideRoot(p.to_path_buf()));
    }
    Ok(())
}
