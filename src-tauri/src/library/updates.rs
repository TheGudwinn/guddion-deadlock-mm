//! Deciding whether a newer file exists for an installed mod.
//!
//! GameBanana gives no reliable "version" for most mods and `_tsDateModified` changes on any edit,
//! so the decision rests on file ids (they only grow) with the version string and MD5 as support.
use crate::sources::ModFile;

/// Archive types the installer can open (also used to pick the file to update to).
pub fn installable(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [".zip", ".7z", ".rar", ".vpk"].iter().any(|e| n.ends_with(e))
}

/// What we remember about the file that is installed.
pub struct InstalledFile<'a> {
    pub file_id: &'a str,
    pub version: Option<&'a str>,
    pub md5: Option<&'a str>,
}

fn numeric(id: &str) -> Option<u64> {
    id.parse().ok()
}

/// The file to update to, or `None` when the installed one is still the latest.
///
/// Flagged when:
/// - the installed file was removed from the mod page (the author replaced it), or
/// - a file with a higher id exists AND the page version differs from the installed version, or
/// - the installed file id still exists but its MD5 changed (re-uploaded in place).
///
/// A higher id alone is not enough: authors often add an alternative variant next to the main file.
pub fn find_update<'a>(
    installed: &InstalledFile,
    remote_version: Option<&str>,
    files: &'a [ModFile],
    installable: impl Fn(&str) -> bool,
) -> Option<&'a ModFile> {
    let candidates: Vec<&ModFile> = files.iter().filter(|f| installable(&f.name)).collect();
    let newest = candidates.iter().copied().max_by_key(|f| numeric(&f.id).unwrap_or(0))?;
    let current = files.iter().find(|f| f.id == installed.file_id);

    match current {
        None => (newest.id != installed.file_id).then_some(newest),
        Some(cur) => {
            let md5_changed = matches!((installed.md5, cur.md5.as_deref()), (Some(a), Some(b)) if !a.eq_ignore_ascii_case(b));
            if md5_changed && installable(&cur.name) {
                return Some(cur);
            }
            let higher = match (numeric(&newest.id), numeric(&installed.file_id)) {
                (Some(n), Some(i)) => n > i,
                _ => newest.id != installed.file_id,
            };
            let version_differs = matches!((installed.version, remote_version), (Some(a), Some(b)) if a.trim() != b.trim());
            (higher && version_differs).then_some(newest)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(id: &str, name: &str, md5: Option<&str>) -> ModFile {
        ModFile { id: id.into(), name: name.into(), size: 1, url: String::new(), md5: md5.map(str::to_string), av_clean: true }
    }
    fn ok(n: &str) -> bool {
        n.ends_with(".zip") || n.ends_with(".vpk") || n.ends_with(".rar") || n.ends_with(".7z")
    }
    fn inst<'a>(id: &'a str, v: Option<&'a str>, md5: Option<&'a str>) -> InstalledFile<'a> {
        InstalledFile { file_id: id, version: v, md5 }
    }

    #[test]
    fn same_file_means_no_update() {
        let files = [f("100", "a.zip", Some("aa"))];
        assert!(find_update(&inst("100", Some("1.0"), Some("aa")), Some("1.0"), &files, ok).is_none());
    }

    #[test]
    fn replaced_file_is_an_update() {
        let files = [f("200", "a_v2.zip", None)];
        assert_eq!(find_update(&inst("100", Some("1.0"), None), Some("2.0"), &files, ok).unwrap().id, "200");
    }

    #[test]
    fn new_file_next_to_old_needs_a_version_change() {
        let files = [f("100", "main.zip", None), f("200", "alt.zip", None)];
        // Alternative variant added, version unchanged: not an update.
        assert!(find_update(&inst("100", Some("1.0"), None), Some("1.0"), &files, ok).is_none());
        // Same layout but the author bumped the version: update to the newest file.
        assert_eq!(find_update(&inst("100", Some("1.0"), None), Some("1.1"), &files, ok).unwrap().id, "200");
        // Unknown versions are not enough evidence.
        assert!(find_update(&inst("100", None, None), None, &files, ok).is_none());
    }

    #[test]
    fn reuploaded_in_place_is_detected_by_md5() {
        let files = [f("100", "a.zip", Some("BB"))];
        assert_eq!(find_update(&inst("100", None, Some("aa")), None, &files, ok).unwrap().id, "100");
        // Case differences in hex are not a change.
        assert!(find_update(&inst("100", None, Some("bb")), None, &files, ok).is_none());
    }

    #[test]
    fn unsupported_files_are_never_offered() {
        let files = [f("200", "setup.exe", None)];
        assert!(find_update(&inst("100", None, None), None, &files, ok).is_none());
        assert!(find_update(&inst("100", None, None), None, &[], ok).is_none());
    }
}
