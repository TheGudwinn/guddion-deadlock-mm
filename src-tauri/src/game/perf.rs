//! Performance configs: sets of `ConVars` values for `gameinfo.gi`.
//!
//! The presets in `data/perf_presets.json` come from the public Deadlock Mod Manager performance
//! catalog (the community configs in it are GPL-3.0 and credited to their authors). Only top level
//! `ConVars` entries are applied. Other sections of those configs (render, scene, particles) are left
//! out on purpose: a wrong value there can stop the game from starting.
//!
//! What is written is easy to take back. New keys go into one marked block right after `ConVars {`;
//! keys the file already has keep their line and get a `// guddion-perf was <old>` note. Removing the
//! overlay restores the file byte for byte, line endings included.
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::LazyLock;

pub const BEGIN: &str = "// ==== Guddion · Performance BEGIN";
pub const END: &str = "// ==== Guddion · Performance END ====";
const WAS: &str = " // guddion-perf was ";

#[derive(Debug, Clone, Deserialize)]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub author: String,
    pub tier: String,
    pub blurb: String,
    pub recommended: bool,
    pub url: Option<String>,
    pub license: Option<String>,
    /// Entries of other sections that are not applied.
    pub skipped: usize,
    pub entries: Vec<(String, String)>,
}

#[derive(Deserialize)]
struct Bundle {
    presets: Vec<Preset>,
}

static PRESETS: LazyLock<Vec<Preset>> = LazyLock::new(|| serde_json::from_str::<Bundle>(include_str!("../../data/perf_presets.json")).map(|b| b.presets).unwrap_or_default());

pub fn presets() -> &'static [Preset] {
    &PRESETS
}

pub fn preset(id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.id == id)
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PerfError {
    #[error("gameinfo.gi has no ConVars block; refusing to edit it")]
    NoConVars,
    #[error("unknown config: {0}")]
    Unknown(String),
    #[error("gameinfo.gi already carries a performance config from another manager; turn it off there first")]
    ForeignOverlay,
    #[error("could not take the other manager's config out cleanly ({0} lines left); gameinfo.gi was not changed")]
    ForeignUnrecognized(usize),
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PerfStatus {
    /// Config written into the file, when there is one.
    pub applied: Option<String>,
    /// Settings the overlay changes (new lines plus edited ones).
    pub settings: usize,
    /// A performance config written by another manager is in the file.
    pub foreign: bool,
}

/// True when another manager's performance overlay is in the file.
pub fn has_foreign(text: &str) -> bool {
    text.contains("dmm-perf") || text.contains("Deadlock Mod Manager · Performance")
}

/// `//` comment removed, ignoring `//` inside quotes. Returns the code part.
fn code_part(line: &str) -> &str {
    let mut in_q = false;
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => in_q = !in_q,
            b'/' if !in_q && b.get(i + 1) == Some(&b'/') => return &line[..i],
            _ => {}
        }
        i += 1;
    }
    line
}

/// Line index of the `{` that opens the top level `ConVars` block, and of its closing `}`.
fn convars_span(lines: &[&str]) -> Option<(usize, usize)> {
    let mut i = 0;
    while i < lines.len() {
        let code = code_part(lines[i]).trim();
        if (code == "ConVars" || code.eq_ignore_ascii_case("\"ConVars\"")) {
            // The opening brace is on this line or the next code line.
            let mut j = i + 1;
            while j < lines.len() && code_part(lines[j]).trim().is_empty() {
                j += 1;
            }
            if j < lines.len() && code_part(lines[j]).trim() == "{" {
                let open = j;
                let mut d = 0i32;
                for (k, l) in lines.iter().enumerate().skip(open) {
                    for c in code_part(l).chars() {
                        match c {
                            '{' => d += 1,
                            '}' => {
                                d -= 1;
                                if d == 0 {
                                    return Some((open, k));
                                }
                            }
                            _ => {}
                        }
                    }
                }
                return None;
            }
        }
        i += 1;
    }
    None
}

/// One `"key"  value` line at the top level of ConVars: the key and the byte range of the value token.
struct Kv {
    line: usize,
    key: String,
    value: std::ops::Range<usize>,
}

fn parse_kv(line: &str) -> Option<(String, std::ops::Range<usize>)> {
    let code = code_part(line);
    let t = code.trim_start();
    let start = code.len() - t.len();
    let b = t.as_bytes();
    if b.first() != Some(&b'"') {
        return None;
    }
    let kend = t[1..].find('"')? + 1;
    let key = t[1..kend].to_string();
    let rest = &t[kend + 1..];
    let r2 = rest.trim_start_matches([' ', '\t']);
    if r2.is_empty() || r2.trim().is_empty() {
        return None; // a key that opens a nested block
    }
    let vstart = start + kend + 1 + (rest.len() - r2.len());
    let vlen = if r2.starts_with('"') { r2[1..].find('"')? + 2 } else { r2.find(|c: char| c.is_whitespace()).unwrap_or(r2.len()) };
    if r2[..vlen].starts_with('{') {
        return None;
    }
    Some((key, vstart..vstart + vlen))
}

fn top_level_kvs(lines: &[&str], open: usize, close: usize) -> Vec<Kv> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    for (i, l) in lines.iter().enumerate().take(close).skip(open) {
        let code = code_part(l);
        if depth == 1 {
            if let Some((key, value)) = parse_kv(l) {
                out.push(Kv { line: i, key, value });
            }
        }
        for c in code.chars() {
            match c {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
        }
    }
    out
}

fn eol_of(line: &str) -> &str {
    if line.ends_with("\r\n") {
        "\r\n"
    } else if line.ends_with('\n') {
        "\n"
    } else {
        ""
    }
}

/// `text` with any earlier overlay of ours taken out.
pub fn remove(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_block = false;
    for line in text.split_inclusive('\n') {
        let t = line.trim_start();
        if !in_block && t.starts_with(BEGIN) {
            in_block = true;
            continue;
        }
        if in_block {
            if t.starts_with(END) {
                in_block = false;
            }
            continue;
        }
        if let Some(pos) = line.find(WAS) {
            // `"key" "new" // guddion-perf was <old>` -> `"key" <old>`
            let eol = eol_of(line);
            let old = line[pos + WAS.len()..].trim_end_matches(['\r', '\n']);
            let head = &line[..pos];
            if let Some((_, range)) = parse_kv(head) {
                out.push_str(&head[..range.start]);
                out.push_str(old);
                out.push_str(&head[range.end..]);
                out.push_str(eol);
                continue;
            }
        }
        out.push_str(line);
    }
    out
}

const F_WAS: &str = " // dmm-perf was ";
const F_ADDED: &str = "// dmm-perf added";
const F_BEGIN: &str = "// ==== Deadlock Mod Manager \u{b7} Performance BEGIN";
const F_END: &str = "// ==== Deadlock Mod Manager \u{b7} Performance END";

/// `text` with the other manager's performance overlay taken out: its marked block is dropped, lines it
/// added are dropped, and values it changed get their old value back. Nothing is returned unless every
/// trace of it is gone, so a half-clean file is never written.
pub fn remove_foreign(text: &str) -> Result<String, PerfError> {
    let mut out = String::with_capacity(text.len());
    let mut in_block = false;
    for line in text.split_inclusive('\n') {
        let t = line.trim_start();
        if !in_block && t.starts_with(F_BEGIN) {
            in_block = true;
            continue;
        }
        if in_block {
            if t.starts_with(F_END) {
                in_block = false;
            }
            continue;
        }
        if line.trim_end().ends_with(F_ADDED) {
            continue;
        }
        if let Some(pos) = line.find(F_WAS) {
            let eol = eol_of(line);
            let old = line[pos + F_WAS.len()..].trim_end_matches(['\r', '\n']);
            let head = &line[..pos];
            if let Some((_, range)) = parse_kv(head).or_else(|| bare_kv(head)) {
                out.push_str(&head[..range.start]);
                out.push_str(old);
                out.push_str(&head[range.end..]);
                out.push_str(eol);
                continue;
            }
        }
        out.push_str(line);
    }
    let left = out.lines().filter(|l| l.contains("dmm-perf")).count() + usize::from(in_block);
    if left > 0 || has_foreign(&out) {
        return Err(PerfError::ForeignUnrecognized(left.max(1)));
    }
    Ok(out)
}

/// `Key value` (key without quotes): the key and the byte range of the value.
fn bare_kv(line: &str) -> Option<(String, std::ops::Range<usize>)> {
    let code = code_part(line);
    let t = code.trim_start();
    let start = code.len() - t.len();
    let kend = t.find([' ', '\t'])?;
    let rest = &t[kend..];
    let r2 = rest.trim_start_matches([' ', '\t']);
    if r2.is_empty() || r2.starts_with('{') {
        return None;
    }
    let vstart = start + kend + (rest.len() - r2.len());
    let vlen = if r2.starts_with('"') { r2[1..].find('"')? + 2 } else { r2.find(|c: char| c.is_whitespace()).unwrap_or(r2.len()) };
    Some((t[..kend].to_string(), vstart..vstart + vlen))
}

pub fn status(text: &str) -> PerfStatus {
    let mut applied = None;
    let mut settings = text.lines().filter(|l| l.contains(WAS)).count();
    let mut in_block = false;
    for l in text.lines() {
        let t = l.trim_start();
        if t.starts_with(BEGIN) {
            in_block = true;
            applied = t.split("config=").nth(1).and_then(|s| s.split(')').next()).map(str::to_string);
        } else if t.starts_with(END) {
            in_block = false;
        } else if in_block && !t.starts_with("//") && !t.is_empty() {
            settings += 1;
        }
    }
    if applied.is_none() && settings > 0 {
        applied = Some(String::new());
    }
    PerfStatus { applied, settings, foreign: has_foreign(text) }
}

/// The file with `preset` applied (any earlier overlay is removed first).
pub fn apply(text: &str, preset: &Preset) -> Result<String, PerfError> {
    if has_foreign(text) {
        return Err(PerfError::ForeignOverlay);
    }
    let clean = remove(text);
    let lines: Vec<&str> = clean.split_inclusive('\n').collect();
    let (open, close) = convars_span(&lines).ok_or(PerfError::NoConVars)?;
    let kvs = top_level_kvs(&lines, open, close);
    let existing: HashSet<&str> = kvs.iter().map(|k| k.key.as_str()).collect();
    let eol = {
        let e = eol_of(lines[open]);
        if e.is_empty() { "\n" } else { e }
    };
    let indent: String = lines.get(open + 1).map(|l| l.chars().take_while(|c| *c == ' ' || *c == '\t').collect()).filter(|s: &String| !s.is_empty()).unwrap_or_else(|| "\t\t".into());

    let mut edits: std::collections::HashMap<usize, Vec<(std::ops::Range<usize>, String)>> = std::collections::HashMap::new();
    for (key, value) in &preset.entries {
        for kv in kvs.iter().filter(|k| &k.key == key) {
            let old = &lines[kv.line][kv.value.clone()];
            let new = format!("\"{value}\"");
            if old != new && old.trim_matches('"') != value {
                edits.entry(kv.line).or_default().push((kv.value.clone(), new));
            }
        }
    }
    let added: Vec<&(String, String)> = preset.entries.iter().filter(|(k, _)| !existing.contains(k.as_str())).collect();

    let mut out = String::with_capacity(clean.len() + preset.entries.len() * 40);
    for (i, line) in lines.iter().enumerate() {
        match edits.get(&i) {
            Some(list) if list.len() == 1 => {
                let (range, new) = &list[0];
                let e = eol_of(line);
                let body = line.trim_end_matches(['\r', '\n']);
                out.push_str(&body[..range.start]);
                out.push_str(new);
                out.push_str(&body[range.end..]);
                out.push_str(WAS);
                out.push_str(&body[range.clone()]);
                out.push_str(e);
            }
            _ => out.push_str(line),
        }
        if i == open && !added.is_empty() {
            // The line before might lack its own terminator only at the very end of the file; not here.
            out.push_str(&format!("{indent}{BEGIN} (config={}) ===={eol}", preset.id));
            out.push_str(&format!("{indent}// {} by {} [guddion-perf]{eol}", preset.name, preset.author));
            for (k, v) in &added {
                out.push_str(&format!("{indent}\"{k}\"\t\t\"{v}\"{eol}"));
            }
            out.push_str(&format!("{indent}{END}{eol}"));
        }
    }
    // A preset that only edits existing keys still needs its marker so the status knows the config.
    if added.is_empty() && !edits.is_empty() {
        let mut marked = String::with_capacity(out.len() + 120);
        for (i, line) in out.split_inclusive('\n').enumerate() {
            marked.push_str(line);
            if i == open {
                marked.push_str(&format!("{indent}{BEGIN} (config={}) ===={eol}{indent}{END}{eol}", preset.id));
            }
        }
        out = marked;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GI: &str = "\"GameInfo\"\r\n{\r\n\tConVars\r\n\t{\t \r\n\t\t\"rate\"\r\n\t\t{\r\n\t\t\t\"max\"\t\"1000000\"\r\n\t\t}\r\n\t\t// Networking\r\n\t\t\"sv_minrate\"\t\"98304\"\r\n\t\t\"fps_max\" 400 // cap\r\n\t}\r\n\tSceneSystem\r\n\t{\r\n\t}\r\n}";

    fn p(entries: &[(&str, &str)]) -> Preset {
        Preset {
            id: "t".into(),
            name: "T".into(),
            author: "A".into(),
            tier: "balanced".into(),
            blurb: String::new(),
            recommended: false,
            url: None,
            license: None,
            skipped: 0,
            entries: entries.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
        }
    }

    #[test]
    fn foreign_overlay_comes_out_and_old_values_return() {
        let dirty = "RenderSystem\r\n{\r\n\t\tIndexBufferPoolSizeMB 128 // dmm-perf was 32\r\n\t\t\"VulkanStagingPMBSizeLimitMB\" \"768\" // dmm-perf was \"384\"\r\n\t\t\"MinDXLevel\"\t\t\"80\" // dmm-perf added\r\n}\r\nConVars\r\n{\r\n\t\t// ==== Deadlock Mod Manager \u{b7} Performance BEGIN (config=x) ====\r\n\t\t// note [dmm-perf]\r\n\t\t\"a\" \"1\"\r\n\t\t// ==== Deadlock Mod Manager \u{b7} Performance END ====\r\n\t\t\"keep\" \"1\"\r\n}\r\n";
        assert!(has_foreign(dirty));
        let clean = remove_foreign(dirty).unwrap();
        assert_eq!(clean, "RenderSystem\r\n{\r\n\t\tIndexBufferPoolSizeMB 32\r\n\t\t\"VulkanStagingPMBSizeLimitMB\" \"384\"\r\n}\r\nConVars\r\n{\r\n\t\t\"keep\" \"1\"\r\n}\r\n");
        assert!(!has_foreign(&clean));
    }

    #[test]
    fn real_file_foreign_overlay_cleans_up() {
        let Ok(path) = std::env::var("GI_SAMPLE") else { return };
        let text = String::from_utf8(std::fs::read(path).unwrap()).unwrap();
        if !has_foreign(&text) {
            return;
        }
        let clean = remove_foreign(&text).expect("clean");
        assert!(!has_foreign(&clean) && !clean.contains("dmm-perf"));
        // the 270-line overlay block goes, the rest of the file stays
        assert!(clean.len() < text.len() && clean.len() * 3 > text.len(), "{} -> {}", text.len(), clean.len());
        let lines: Vec<&str> = clean.split_inclusive('\n').collect();
        assert!(convars_span(&lines).is_some());
        // our own overlay can now be applied on top
        assert!(apply(&clean, &presets()[0]).is_ok());
    }

    #[test]
    fn foreign_overlay_with_unknown_marker_is_left_alone() {
        assert!(matches!(remove_foreign("ConVars\n{\n\"a\" \"1\" // dmm-perf something new\n}\n"), Err(PerfError::ForeignUnrecognized(_))));
    }

    #[test]
    fn apply_then_remove_is_byte_identical() {
        let preset = p(&[("sv_minrate", "1"), ("fps_max", "0"), ("new_key", "true")]);
        let applied = apply(GI, &preset).unwrap();
        assert!(applied.contains("\"new_key\"\t\t\"true\""), "{applied}");
        assert!(applied.contains("\"sv_minrate\"\t\"1\" // guddion-perf was \"98304\""), "{applied}");
        assert!(applied.contains("\"fps_max\" \"0\" // cap // guddion-perf was 400") || applied.contains("\"fps_max\" \"0\""), "{applied}");
        assert_eq!(remove(&applied), GI);
        let st = status(&applied);
        assert_eq!(st.applied.as_deref(), Some("t"));
        assert_eq!(st.settings, 3);
    }

    #[test]
    fn switching_presets_does_not_stack() {
        let a = apply(GI, &p(&[("new_key", "1")])).unwrap();
        let mut second = p(&[("other", "2")]);
        second.id = "u".into();
        let b = apply(&a, &second).unwrap();
        assert!(!b.contains("new_key") && b.contains("other"));
        assert_eq!(remove(&b), GI);
    }

    #[test]
    fn nested_keys_are_not_touched_and_only_edits_still_marked() {
        let applied = apply(GI, &p(&[("max", "5"), ("sv_minrate", "7")])).unwrap();
        assert!(applied.contains("\"max\"\t\"1000000\""), "a key inside `rate` stays: {applied}");
        assert!(applied.contains("config=t"));
        assert_eq!(remove(&applied), GI);
    }

    #[test]
    fn no_convars_block_is_an_error() {
        assert_eq!(apply("\"GameInfo\"\n{\n}\n", &p(&[("a", "1")])), Err(PerfError::NoConVars));
    }

    #[test]
    fn bundled_presets_load_and_apply_to_a_plain_file() {
        assert!(presets().len() >= 5);
        for pr in presets() {
            assert!(!pr.entries.is_empty(), "{}", pr.id);
            let out = apply(GI, pr).unwrap();
            assert_eq!(remove(&out), GI, "{}", pr.id);
        }
    }

    /// Runs on a copy of the real file when `GI_SAMPLE` points at one (skipped otherwise).
    #[test]
    fn real_game_file_round_trip() {
        let Ok(path) = std::env::var("GI_SAMPLE") else { return };
        let text = String::from_utf8(std::fs::read(path).unwrap()).unwrap();
        let base = remove(&text);
        if has_foreign(&base) {
            assert_eq!(apply(&base, &presets()[0]), Err(PerfError::ForeignOverlay));
            return;
        }
        for pr in presets() {
            let out = apply(&base, pr).unwrap();
            assert_ne!(out, base, "{}", pr.id);
            assert_eq!(remove(&out), base, "{}", pr.id);
            assert_eq!(status(&out).applied.as_deref(), Some(pr.id.as_str()));
            assert!(crate::install::gameinfo::ensure_addons_path(&out) == crate::install::gameinfo::ensure_addons_path(&base), "SearchPaths result must not change: {}", pr.id);
        }
    }
}
