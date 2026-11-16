//! Works out which catalog mod a file installed by another manager belongs to, without hashing or
//! asking any server. Deadlock Mod Manager leaves two local traces:
//!
//! * the file name: `<GameBanana id>_pakNN_dir.vpk` (sound mods use `snd-<id>`), and
//! * a manifest `.dmm.json` in the addons folder that lists each mod's files.
//!
//! Both are only hints written by another program, so the result is just an id to look up in the
//! catalog; nothing here is trusted for anything but display.
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

const MANIFEST: &str = ".dmm.json";

/// `569650_pak06_dir.vpk` -> `569650`, `snd-12_pak01_dir.vpk` -> `snd-12`. Anything else -> `None`.
pub fn id_from_file_name(name: &str) -> Option<String> {
    let name = name.strip_suffix(".disabled").unwrap_or(name);
    let prefix = name.strip_suffix("_dir.vpk")?.rsplit_once("_pak")?.0;
    is_catalog_id(prefix).then(|| prefix.to_string())
}

/// Whether the game loads this file: only `pak##_dir.vpk`. Deadlock Mod Manager switches a mod off by
/// giving its files the mod id as a prefix (`569650_pak06_dir.vpk`), which the game then ignores.
pub fn is_loaded_name(name: &str) -> bool {
    name.strip_prefix("pak")
        .and_then(|r| r.strip_suffix("_dir.vpk"))
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// A GameBanana mod id (`123`) or sound id (`snd-123`).
pub fn is_catalog_id(s: &str) -> bool {
    let digits = s.strip_prefix("snd-").unwrap_or(s);
    !digits.is_empty() && digits.len() <= 12 && digits.bytes().all(|b| b.is_ascii_digit())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    #[serde(default)]
    mods: BTreeMap<String, Entry>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Entry {
    current_vpks: Vec<String>,
    disabled_vpks: Vec<String>,
}

/// File name -> catalog mod id, from the manager's `.dmm.json`. A file claimed by two entries is
/// left out (neither owner can be trusted), and so are entries that are not catalog mods (local imports).
/// A missing or unreadable manifest simply yields nothing.
pub fn manifest_owners(addons_dir: &Path) -> HashMap<String, String> {
    let Ok(text) = std::fs::read_to_string(addons_dir.join(MANIFEST)) else { return HashMap::new() };
    let Ok(m) = serde_json::from_str::<Manifest>(&text) else { return HashMap::new() };
    owners_of(&m)
}

fn owners_of(m: &Manifest) -> HashMap<String, String> {
    let mut owners: HashMap<String, Option<String>> = HashMap::new();
    for (id, entry) in &m.mods {
        for file in entry.current_vpks.iter().chain(&entry.disabled_vpks) {
            let file = file.rsplit(['/', '\\']).next().unwrap_or(file).to_string();
            owners.entry(file).and_modify(|o| *o = None).or_insert_with(|| is_catalog_id(id).then(|| id.clone()));
        }
    }
    owners.into_iter().filter_map(|(k, v)| v.map(|v| (k, v))).collect()
}

/// File name -> catalog id, remembered from the switches this manager made itself (its journal records
/// the mod id in the batch name). The other manager may rewrite or drop its own manifest when it sees the
/// renames, so this keeps a mod identified after it was switched off and on again here.
pub fn journal_owners(entries: &[crate::install::journal::Entry]) -> HashMap<String, String> {
    use crate::install::journal::Op;
    fn file(p: &Path) -> Option<String> {
        p.file_name().map(|n| n.to_string_lossy().into_owned())
    }
    let mut owners: HashMap<String, String> = HashMap::new();
    for e in entries {
        let Some(tag) = e.batch.strip_prefix("foreign-g").and_then(|r| r.split('-').next()) else { continue };
        if !is_catalog_id(tag) {
            continue;
        }
        if e.rolled_back {
            // An undone switch-off put `<id>_pak##_dir.vpk` back to `pak##_dir.vpk`: that file is the mod's.
            if let Op::Rename { from, to } = &e.op {
                if let (Some(f), Some(t)) = (file(from), file(to)) {
                    if t.starts_with(&format!("{tag}_")) && is_loaded_name(&f) {
                        owners.insert(f, tag.to_string());
                    }
                }
            }
            continue;
        }
        if let Op::Rename { from, to } = &e.op {
            if let Some(f) = file(from) {
                owners.remove(&f); // that name no longer exists
            }
            if let Some(t) = file(to) {
                owners.insert(t.strip_suffix(".disabled").unwrap_or(&t).to_string(), tag.to_string());
            }
        }
    }
    owners
}

/// The catalog id of `file_name`: the manager's own naming first, then its manifest.
pub fn identify(file_name: &str, owners: &HashMap<String, String>) -> Option<String> {
    id_from_file_name(file_name).or_else(|| owners.get(file_name.strip_suffix(".disabled").unwrap_or(file_name)).cloned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_id_from_the_file_name() {
        assert_eq!(id_from_file_name("569650_pak06_dir.vpk").as_deref(), Some("569650"));
        assert_eq!(id_from_file_name("snd-12_pak01_dir.vpk").as_deref(), Some("snd-12"));
        assert_eq!(id_from_file_name("659083_pak67_dir.vpk.disabled").as_deref(), Some("659083"));
        for no in ["pak01_dir.vpk", "ddmm5_pak01_dir.vpk", "abc_pak01_dir.vpk", "123_pak01.vpk", "_pak01_dir.vpk", "snd-_pak01_dir.vpk", "1234567890123_pak01_dir.vpk"] {
            assert_eq!(id_from_file_name(no), None, "{no}");
        }
    }

    #[test]
    fn only_pak_number_names_are_loaded() {
        for yes in ["pak01_dir.vpk", "pak67_dir.vpk", "pak100_dir.vpk"] {
            assert!(is_loaded_name(yes), "{yes}");
        }
        for no in ["569650_pak06_dir.vpk", "pak_dir.vpk", "pak01_dir.vpk.disabled", "pakx1_dir.vpk", "mypak01_dir.vpk", "pak01.vpk", "ddmm3_pak01_dir.vpk"] {
            assert!(!is_loaded_name(no), "{no}");
        }
    }

    #[test]
    fn manifest_assigns_files_and_drops_ambiguous_or_local_ones() {
        let m: Manifest = serde_json::from_str(
            r#"{"version":3,"mods":{
                "650634":{"enabled":true,"currentVpks":["pak01_dir.vpk"],"disabledVpks":[]},
                "671416":{"enabled":false,"currentVpks":[],"disabledVpks":["sub/pak02_dir.vpk"]},
                "local-abc":{"enabled":true,"currentVpks":["pak03_dir.vpk"]},
                "111":{"currentVpks":["pak04_dir.vpk"]},
                "222":{"currentVpks":["pak04_dir.vpk"]}
            }}"#,
        )
        .unwrap();
        let o = owners_of(&m);
        assert_eq!(o.get("pak01_dir.vpk").map(String::as_str), Some("650634"));
        assert_eq!(o.get("pak02_dir.vpk").map(String::as_str), Some("671416"));
        assert_eq!(o.get("pak03_dir.vpk"), None, "local imports are not catalog mods");
        assert_eq!(o.get("pak04_dir.vpk"), None, "claimed twice");
    }

    #[test]
    fn our_own_journal_keeps_a_switched_mod_identified() {
        use crate::install::journal::{Entry, Op};
        let e = |id: u64, batch: &str, from: &str, to: &str, rolled_back: bool| Entry {
            id,
            ts: 0,
            batch: batch.to_string(),
            op: Op::Rename { from: from.into(), to: to.into() },
            rolled_back,
        };
        let log = vec![
            e(1, "foreign-g691863-10", "D:/a/pak30_dir.vpk", "D:/a/691863_pak30_dir.vpk", false),
            e(2, "foreign-g691863-11", "D:/a/691863_pak30_dir.vpk", "D:/a/pak30_dir.vpk", false),
            e(3, "foreign-g777-12", "D:/a/pak01_dir.vpk", "D:/a/777_pak01_dir.vpk", true), // undone: pak01 is still 777's
            e(4, "install-5-13", "D:/a/x", "D:/a/pak40_dir.vpk", false),                    // not a foreign switch
            e(5, "foreign-gx-14", "D:/a/pak02_dir.vpk", "D:/a/pak02_dir.vpk.disabled", false), // no catalog id
        ];
        let o = journal_owners(&log);
        assert_eq!(o.get("pak30_dir.vpk").map(String::as_str), Some("691863"));
        assert_eq!(o.get("691863_pak30_dir.vpk"), None, "the old name is gone");
        assert_eq!(o.get("pak01_dir.vpk").map(String::as_str), Some("777"), "an undone switch-off still says whose it is");
        assert_eq!(o.len(), 2, "{o:?}");
    }

    #[test]
    fn name_wins_over_manifest_and_missing_manifest_is_fine() {
        let owners = HashMap::from([("pak01_dir.vpk".to_string(), "650634".to_string())]);
        assert_eq!(identify("pak01_dir.vpk", &owners).as_deref(), Some("650634"));
        assert_eq!(identify("999_pak01_dir.vpk", &owners).as_deref(), Some("999"));
        assert_eq!(identify("pak09_dir.vpk", &owners), None);
        assert!(manifest_owners(Path::new("Z:/does/not/exist")).is_empty());
    }
}
