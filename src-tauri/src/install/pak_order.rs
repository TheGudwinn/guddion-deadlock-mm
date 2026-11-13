//! Naming and load order of addon VPKs.
//!
//! Files are named `<PREFIX><mod_id>_pakNN_dir.vpk`. The prefix keeps our files distinguishable from
//! other managers' files already present in `addons/`; NN is the load slot (01..99).
//!
//! UNVERIFIED: which slot wins when two VPKs contain the same file. We assume the LOWER slot wins
//! (position 0 = highest priority -> slot 01). Verify on a real game before relying on it.

pub const PREFIX: &str = "ddmm";
pub const MAX_SLOTS: usize = 99;
pub const DISABLED_SUFFIX: &str = ".disabled";

pub fn addon_name(mod_id: i64, slot: u8) -> String {
    format!("{PREFIX}{mod_id}_pak{slot:02}_dir.vpk")
}

pub fn disabled_name(name: &str) -> String {
    format!("{name}{DISABLED_SUFFIX}")
}

/// True for files created by this app (enabled or disabled).
pub fn is_ours(file_name: &str) -> bool {
    let n = file_name.strip_suffix(DISABLED_SUFFIX).unwrap_or(file_name);
    n.starts_with(PREFIX) && n.ends_with("_dir.vpk") && n.contains("_pak")
}

/// Slot number parsed from one of our file names.
pub fn slot_of(file_name: &str) -> Option<u8> {
    if !is_ours(file_name) {
        return None;
    }
    let rest = file_name.split("_pak").nth(1)?;
    rest.get(..2)?.parse().ok()
}

/// Database id of the mod a file of ours belongs to (`ddmm<id>_pakNN_dir.vpk` -> id).
pub fn mod_id_of(file_name: &str) -> Option<i64> {
    if !is_ours(file_name) {
        return None;
    }
    file_name.strip_prefix(PREFIX)?.split("_pak").next()?.parse().ok()
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
        assert_eq!(addon_name(42, 3), "ddmm42_pak03_dir.vpk");
        assert!(is_ours("ddmm42_pak03_dir.vpk"));
        assert!(is_ours(&disabled_name("ddmm42_pak03_dir.vpk")));
        assert!(!is_ours("659083_pak67_dir.vpk")); // another manager's file
        assert!(!is_ours("pak01_dir.vpk"));
    }

    #[test]
    fn parses_and_allocates_slots() {
        assert_eq!(slot_of("ddmm42_pak07_dir.vpk.disabled"), Some(7));
        assert_eq!(slot_of("659083_pak67_dir.vpk"), None);
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
