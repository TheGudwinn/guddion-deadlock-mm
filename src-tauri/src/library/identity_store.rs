//! Remembers which catalog mod an addon file is, so a mod stays recognised even when the other manager
//! renames its files or rewrites (or loses) its manifest. The file is fingerprinted by its size and the
//! number of entries inside; both are read from the file itself, so a rename does not change them.
//! If two different mods ever share a fingerprint it is marked ambiguous and never used.
use crate::install::gameinfo::write_atomic;
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

#[derive(Default, Debug)]
pub struct IdentityStore {
    map: BTreeMap<String, String>,
    dirty: bool,
}

impl IdentityStore {
    /// A missing or unreadable file is an empty store: this is only a cache of what was seen before.
    pub fn load(path: &Path) -> Self {
        let map = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Self { map, dirty: false }
    }

    pub fn key(size: u64, entries: Option<usize>) -> String {
        format!("{size}:{}", entries.map_or(-1, |n| n as i64))
    }

    /// The remembered catalog id for this fingerprint, unless it is ambiguous.
    pub fn lookup(&self, key: &str) -> Option<&str> {
        self.map.get(key).map(String::as_str).filter(|id| !id.is_empty())
    }

    pub fn remember(&mut self, key: &str, id: &str) {
        match self.map.get(key).map(String::as_str) {
            None => {
                self.map.insert(key.to_string(), id.to_string());
                self.dirty = true;
            }
            Some(old) if old == id || old.is_empty() => {}
            Some(_) => {
                self.map.insert(key.to_string(), String::new()); // two mods, one fingerprint: do not trust it
                self.dirty = true;
            }
        }
    }

    /// Forget fingerprints of files that are no longer in the folder.
    pub fn retain_present(&mut self, present: &HashSet<String>) {
        let before = self.map.len();
        self.map.retain(|k, _| present.contains(k));
        self.dirty |= self.map.len() != before;
    }

    pub fn save(&mut self, path: &Path) {
        if !self.dirty {
            return;
        }
        if let Ok(text) = serde_json::to_string(&self.map) {
            if write_atomic(path, text.as_bytes()).is_ok() {
                self.dirty = false;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembers_and_marks_conflicts_ambiguous() {
        let mut s = IdentityStore::default();
        let k = IdentityStore::key(589_395_688, Some(312));
        assert_eq!(s.lookup(&k), None);
        s.remember(&k, "691863");
        assert_eq!(s.lookup(&k), Some("691863"));
        s.remember(&k, "691863");
        s.remember(&k, "42"); // a different mod with the same fingerprint
        assert_eq!(s.lookup(&k), None, "ambiguous fingerprints are never used");
        s.remember(&k, "691863");
        assert_eq!(s.lookup(&k), None, "and stay ambiguous");
    }

    #[test]
    fn prunes_files_that_are_gone_and_roundtrips_through_disk() {
        let dir = std::env::temp_dir().join(format!("ddmm-id-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("foreign_ids.json");
        let (a, b) = (IdentityStore::key(1, Some(1)), IdentityStore::key(2, None));
        let mut s = IdentityStore::load(&path);
        s.remember(&a, "10");
        s.remember(&b, "20");
        s.save(&path);

        let mut s = IdentityStore::load(&path);
        assert_eq!(s.lookup(&a), Some("10"));
        s.retain_present(&HashSet::from([a.clone()]));
        s.save(&path);
        let s = IdentityStore::load(&path);
        assert_eq!((s.lookup(&a), s.lookup(&b)), (Some("10"), None));

        std::fs::write(&path, "not json").unwrap();
        assert_eq!(IdentityStore::load(&path).lookup(&a), None, "a corrupt file is an empty store");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
