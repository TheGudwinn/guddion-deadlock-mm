//! Profiles: named sets of mods (enabled state + order), portable as JSON files.
pub mod store;

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const FORMAT: &str = "deadlock-mm-profile";
pub const VERSION: u32 = 1;
pub const MAX_FILE_BYTES: usize = 1 << 20;
pub const MAX_MODS: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileEntry {
    pub source: String,
    pub remote_id: String,
    pub name: String,
    pub version: Option<String>,
    pub enabled: bool,
    pub position: i32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProfileFile {
    pub format: String,
    pub version: u32,
    pub name: String,
    pub mods: Vec<ProfileEntry>,
}

pub fn to_file(name: &str, mods: Vec<ProfileEntry>) -> ProfileFile {
    ProfileFile { format: FORMAT.into(), version: VERSION, name: name.into(), mods }
}

fn valid_token(s: &str, max: usize, extra: &[char]) -> bool {
    !s.is_empty() && s.len() <= max && s.chars().all(|c| c.is_ascii_alphanumeric() || extra.contains(&c))
}

pub fn clean_name(name: &str) -> Result<String, String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("empty_name".into());
    }
    if n.chars().count() > 100 || n.chars().any(|c| c.is_control()) {
        return Err("invalid_name".into());
    }
    Ok(n.to_string())
}

/// Parse and validate an imported profile. Untrusted input: strict limits, nothing is executed or
/// resolved to a path; entries are only ever matched against already installed mods by id.
pub fn parse_profile_file(bytes: &[u8]) -> Result<ProfileFile, String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err("file_too_large".into());
    }
    let mut f: ProfileFile = serde_json::from_slice(bytes).map_err(|e| format!("not_a_profile:{e}"))?;
    if f.format != FORMAT {
        return Err("not_a_profile:wrong format".into());
    }
    if f.version != VERSION {
        return Err(format!("unsupported_version:{}", f.version));
    }
    f.name = clean_name(&f.name)?;
    if f.mods.len() > MAX_MODS {
        return Err("too_many_mods".into());
    }
    let mut seen = HashSet::new();
    for m in &mut f.mods {
        if !valid_token(&m.source, 32, &['_', '-']) || !valid_token(&m.remote_id, 64, &['_', '-']) {
            return Err("invalid_entry".into());
        }
        if m.name.chars().count() > 300 || m.name.chars().any(|c| c.is_control()) {
            return Err("invalid_entry".into());
        }
        if m.version.as_ref().is_some_and(|v| v.len() > 64 || v.chars().any(|c| c.is_control())) {
            return Err("invalid_entry".into());
        }
        if !seen.insert((m.source.clone(), m.remote_id.clone())) {
            return Err("duplicate_entry".into());
        }
    }
    // Order comes from the file's positions, then is normalized to 0..n.
    f.mods.sort_by_key(|m| m.position);
    for (i, m) in f.mods.iter_mut().enumerate() {
        m.position = i as i32;
    }
    Ok(f)
}

#[derive(Debug, Clone)]
pub struct InstalledRef {
    pub id: i64,
    pub source: String,
    pub remote_id: String,
    pub enabled: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub enable: Vec<i64>,
    pub disable: Vec<i64>,
    /// In the profile but not installed here.
    pub missing: Vec<ProfileEntry>,
}

/// Which installed mods must change state to match the profile. Installed mods the profile does
/// not mention are switched off, so the profile fully defines what is active.
pub fn plan_apply(entries: &[ProfileEntry], installed: &[InstalledRef]) -> Plan {
    let mut plan = Plan::default();
    for e in entries {
        match installed.iter().find(|i| i.source == e.source && i.remote_id == e.remote_id) {
            None => plan.missing.push(e.clone()),
            Some(i) if i.enabled != e.enabled => {
                if e.enabled { plan.enable.push(i.id) } else { plan.disable.push(i.id) }
            }
            Some(_) => {}
        }
    }
    for i in installed {
        let listed = entries.iter().any(|e| e.source == i.source && e.remote_id == i.remote_id);
        if !listed && i.enabled {
            plan.disable.push(i.id);
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, enabled: bool, pos: i32) -> ProfileEntry {
        ProfileEntry { source: "gamebanana".into(), remote_id: id.into(), name: format!("mod {id}"), version: None, enabled, position: pos }
    }
    fn inst(id: i64, remote: &str, enabled: bool) -> InstalledRef {
        InstalledRef { id, source: "gamebanana".into(), remote_id: remote.into(), enabled }
    }

    #[test]
    fn plan_switches_only_what_differs() {
        let entries = [entry("1", true, 0), entry("2", false, 1), entry("3", true, 2), entry("9", true, 3)];
        let installed = [inst(10, "1", true), inst(20, "2", true), inst(30, "3", false), inst(40, "4", true), inst(50, "5", false)];
        let p = plan_apply(&entries, &installed);
        assert_eq!(p.enable, [30]);
        assert_eq!(p.disable, [20, 40]); // 2 is listed as off; 4 is unlisted and on; 5 is already off
        assert_eq!(p.missing.len(), 1);
        assert_eq!(p.missing[0].remote_id, "9");
    }

    #[test]
    fn matching_state_is_a_no_op() {
        let p = plan_apply(&[entry("1", true, 0)], &[inst(1, "1", true)]);
        assert_eq!(p, Plan::default());
    }

    #[test]
    fn export_import_roundtrip_normalizes_order() {
        let f = to_file("Ranked", vec![entry("2", true, 5), entry("1", false, 1)]);
        let parsed = parse_profile_file(&serde_json::to_vec(&f).unwrap()).unwrap();
        assert_eq!(parsed.name, "Ranked");
        assert_eq!(parsed.mods.iter().map(|m| (m.remote_id.as_str(), m.position)).collect::<Vec<_>>(), [("1", 0), ("2", 1)]);
    }

    #[test]
    fn rejects_hostile_or_broken_files() {
        let ok = |f: &ProfileFile| serde_json::to_vec(f).unwrap();
        let mut f = to_file("x", vec![entry("1", true, 0)]);
        f.mods[0].remote_id = "../../etc/passwd".into();
        assert_eq!(parse_profile_file(&ok(&f)).unwrap_err(), "invalid_entry");

        let mut f = to_file("x", vec![entry("1", true, 0), entry("1", false, 1)]);
        assert_eq!(parse_profile_file(&ok(&f)).unwrap_err(), "duplicate_entry");
        f.mods.truncate(1);
        f.version = 99;
        assert!(parse_profile_file(&ok(&f)).unwrap_err().starts_with("unsupported_version"));

        assert_eq!(parse_profile_file(&ok(&to_file("   ", vec![]))).unwrap_err(), "empty_name");
        assert!(parse_profile_file(b"{\"hello\":1}").unwrap_err().starts_with("not_a_profile"));
        assert_eq!(parse_profile_file(&vec![b' '; MAX_FILE_BYTES + 1]).unwrap_err(), "file_too_large");
        let many = to_file("x", (0..=MAX_MODS as i32).map(|i| entry(&i.to_string(), true, i)).collect());
        assert_eq!(parse_profile_file(&ok(&many)).unwrap_err(), "too_many_mods");
    }
}
