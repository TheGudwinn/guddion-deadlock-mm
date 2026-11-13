use std::collections::BTreeMap;

pub struct ModFiles {
    pub mod_id: i64,
    /// (path inside VPK, crc32)
    pub files: Vec<(String, u32)>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Conflict {
    pub path: String,
    /// Mods providing this file, in the order given.
    pub mods: Vec<i64>,
    /// All copies are byte-identical (same CRC), so the conflict is harmless.
    pub identical: bool,
}

pub fn find_conflicts(mods: &[ModFiles]) -> Vec<Conflict> {
    let mut by_path: BTreeMap<&str, Vec<(i64, u32)>> = BTreeMap::new();
    for m in mods {
        for (p, crc) in &m.files {
            by_path.entry(p.as_str()).or_default().push((m.mod_id, *crc));
        }
    }
    by_path
        .into_iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|(p, v)| Conflict {
            path: p.to_string(),
            identical: v.iter().all(|(_, c)| *c == v[0].1),
            mods: v.into_iter().map(|(m, _)| m).collect(),
        })
        .collect()
}

/// The mod whose file is used: the first in `priority` (highest priority first) that provides it.
pub fn winner(conflict: &Conflict, priority: &[i64]) -> Option<i64> {
    priority.iter().copied().find(|m| conflict.mods.contains(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_shared_paths_and_identical_copies() {
        let m = |id, f: &[(&str, u32)]| ModFiles { mod_id: id, files: f.iter().map(|(p, c)| (p.to_string(), *c)).collect() };
        let c = find_conflicts(&[m(1, &[("a", 1), ("b", 5)]), m(2, &[("a", 2), ("b", 5), ("c", 9)]), m(3, &[("z", 0)])]);
        assert_eq!(c.len(), 2);
        assert!(!c[0].identical && c[0].path == "a");
        assert!(c[1].identical && c[1].path == "b");
        assert_eq!(winner(&c[0], &[2, 1]), Some(2));
        assert_eq!(winner(&c[0], &[3]), None);
    }
}
