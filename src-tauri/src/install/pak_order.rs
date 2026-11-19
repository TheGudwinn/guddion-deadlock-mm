//! Naming and load order of addon VPKs.
//!
//! The game mounts only files named exactly `pakNN_dir.vpk` (NN = the load slot, 01..99) from `addons/`; a
//! file with any other name, an id or manager prefix included, is ignored. That is how other managers switch a
//! mod off. Our files are therefore plain `pakNN_dir.vpk` files and which of them are ours is recorded in the
//! database (`set_owned`). Earlier versions named them `ddmm<id>_pakNN_dir.vpk`, which the game never loaded;
//! those names are still recognised (`is_legacy`) so they can be renamed.
//!
//! UNVERIFIED: which slot wins when two VPKs contain the same file. We assume the LOWER slot wins
//! (position 0 = highest priority -> slot 01). Verify on a real game before relying on it.
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

pub const PREFIX: &str = "ddmm";
pub const MAX_SLOTS: usize = 99;
pub const DISABLED_SUFFIX: &str = ".disabled";

/// File name (enabled form) -> database id of the mod, for every file this app installed.
static OWNED: OnceLock<RwLock<HashMap<String, i64>>> = OnceLock::new();

fn owned() -> &'static RwLock<HashMap<String, i64>> {
    OWNED.get_or_init(Default::default)
}

/// Replaces what is known about which files are ours; called whenever the database is read.
pub fn set_owned(files: HashMap<String, i64>) {
    if let Ok(mut w) = owned().write() {
        *w = files;
    }
}

fn base_of(name: &str) -> &str {
    name.strip_suffix(DISABLED_SUFFIX).unwrap_or(name)
}

fn owned_id(name: &str) -> Option<i64> {
    owned().read().ok()?.get(base_of(name)).copied()
}

pub fn addon_name(_mod_id: i64, slot: u8) -> String {
    format!("pak{slot:02}_dir.vpk")
}

pub fn disabled_name(name: &str) -> String {
    format!("{name}{DISABLED_SUFFIX}")
}

/// A name of the first versions (`ddmm<id>_pakNN_dir.vpk`), which the game never loaded.
pub fn is_legacy(file_name: &str) -> bool {
    let n = base_of(file_name);
    n.starts_with(PREFIX) && n.ends_with("_dir.vpk") && n.contains("_pak")
}

/// True for files installed by this app (enabled or disabled).
pub fn is_ours(file_name: &str) -> bool {
    is_legacy(file_name) || owned_id(file_name).is_some()
}

/// Load slot in a file name: `pak07_dir.vpk`, `ddmm42_pak07_dir.vpk`, `123_pak07_dir.vpk.disabled`.
pub fn slot_of(file_name: &str) -> Option<u8> {
    let base = base_of(file_name).strip_suffix("_dir.vpk")?;
    let digits = base.rsplit_once("_pak").map(|(_, d)| d).or_else(|| base.strip_prefix("pak"))?;
    if digits.len() == 2 { digits.parse().ok() } else { None }
}

/// Database id of the mod a file of ours belongs to.
pub fn mod_id_of(file_name: &str) -> Option<i64> {
    if is_legacy(file_name) {
        return base_of(file_name).strip_prefix(PREFIX)?.split("_pak").next()?.parse().ok();
    }
    owned_id(file_name)
}

/// Lowest free slot given the slots already used.
pub fn first_free_slot(used: &[u8]) -> Option<u8> {
    (1..=MAX_SLOTS as u8).find(|s| !used.contains(s))
}

/// Slot for a priority position (0 = highest priority).
pub fn slot_for_position(position: usize) -> Option<u8> {
    (position < MAX_SLOTS).then(|| (position + 1) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_ownership() {
        // the game only loads plain pakNN_dir.vpk names
        assert_eq!(addon_name(42, 3), "pak03_dir.vpk");
        // the first versions' names are still recognised so they can be renamed
        assert!(is_legacy("ddmm42_pak03_dir.vpk") && is_ours("ddmm42_pak03_dir.vpk"));
        assert!(is_ours(&disabled_name("ddmm42_pak03_dir.vpk")));
        assert!(!is_ours("659083_pak67_dir.vpk")); // another manager's file
        assert!(!is_ours("pak01_dir.vpk")); // plain names are ours only when the database says so
    }

    #[test]
    fn plain_names_are_ours_when_recorded() {
        set_owned(HashMap::from([("pak97_dir.vpk".to_string(), 7)]));
        assert!(is_ours("pak97_dir.vpk") && is_ours("pak97_dir.vpk.disabled"));
        assert_eq!(mod_id_of("pak97_dir.vpk.disabled"), Some(7));
        assert!(!is_ours("pak96_dir.vpk"));
        set_owned(HashMap::new());
        assert!(!is_ours("pak97_dir.vpk"));
    }

    #[test]
    fn parses_and_allocates_slots() {
        assert_eq!(slot_of("ddmm42_pak07_dir.vpk.disabled"), Some(7));
        assert_eq!(slot_of("659083_pak67_dir.vpk"), Some(67));
        assert_eq!(slot_of("pak05_dir.vpk"), Some(5));
        assert_eq!(slot_of("pak5_dir.vpk"), None);
        assert_eq!(slot_of("other.vpk"), None);
        assert_eq!(mod_id_of("ddmm42_pak07_dir.vpk.disabled"), Some(42));
        assert_eq!(mod_id_of("659083_pak67_dir.vpk"), None);
        assert_eq!(first_free_slot(&[1, 2, 4]), Some(3));
        assert_eq!(first_free_slot(&[]), Some(1));
    }

    #[test]
    fn slots_are_bounded() {
        assert_eq!(slot_for_position(0), Some(1));
        assert_eq!(slot_for_position(98), Some(99));
        assert_eq!(slot_for_position(99), None);
    }
}
