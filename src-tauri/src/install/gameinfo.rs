//! Minimal, surgical edit of `gameinfo.gi`: make sure `Game citadel/addons` is listed in
//! SearchPaths before the `Game citadel` line. Nothing else in the file is touched, so edits made
//! by other managers (deadworks, Deadlock Mod Manager) are preserved.
//!
//! The file is KeyValues text. It is tokenized properly (line and block comments, quoted strings,
//! braces anywhere on a line) instead of being scanned line by line, and the edit is refused
//! (`Unrecognized`) whenever the layout is not exactly what we can patch with certainty.
use std::io::Write;
use std::path::Path;

#[derive(Debug, PartialEq, Eq)]
pub enum PatchResult {
    AlreadyPresent,
    Patched(String),
    /// SearchPaths block or the `Game citadel` anchor was not found; do not edit.
    Unrecognized,
}

const ADDONS: &str = "citadel/addons";

#[derive(Debug, PartialEq, Eq)]
enum Tok {
    Open,
    Close,
    Word(String),
}

struct Token {
    tok: Tok,
    line: usize,
}

/// Split KeyValues text into tokens. `None` = malformed (unterminated comment or string).
fn lex(text: &str) -> Option<Vec<Token>> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let (mut i, mut line) = (0usize, 0usize);
    while i < b.len() {
        match b[i] {
            b'\n' => {
                line += 1;
                i += 1;
            }
            b' ' | b'\t' | b'\r' => i += 1,
            b'/' if b.get(i + 1) == Some(&b'/') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                loop {
                    if i + 1 >= b.len() {
                        return None; // unterminated block comment
                    }
                    if b[i] == b'*' && b[i + 1] == b'/' {
                        i += 2;
                        break;
                    }
                    if b[i] == b'\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
            b'{' => {
                out.push(Token { tok: Tok::Open, line });
                i += 1;
            }
            b'}' => {
                out.push(Token { tok: Tok::Close, line });
                i += 1;
            }
            b'"' => {
                i += 1;
                let start = i;
                while i < b.len() && b[i] != b'"' && b[i] != b'\n' {
                    i += 1;
                }
                if i >= b.len() || b[i] != b'"' {
                    return None; // unterminated string
                }
                out.push(Token { tok: Tok::Word(text[start..i].to_string()), line });
                i += 1;
            }
            _ => {
                let start = i;
                while i < b.len()
                    && !matches!(b[i], b' ' | b'\t' | b'\r' | b'\n' | b'{' | b'}' | b'"')
                    && !(b[i] == b'/' && matches!(b.get(i + 1), Some(b'/') | Some(b'*')))
                {
                    i += 1;
                }
                out.push(Token { tok: Tok::Word(text[start..i].to_string()), line });
            }
        }
    }
    Some(out)
}

fn same_path(value: &str, want: &str) -> bool {
    let v = value.replace('\\', "/");
    v.trim_end_matches('/').eq_ignore_ascii_case(want)
}

/// What one SearchPaths block contains.
struct Block {
    has_addons: bool,
    /// Line index of the first clean `Game citadel` line (exactly those two words, nothing else).
    anchor: Option<usize>,
}

/// Parse every `SearchPaths { ... }` block. `None` = the block structure is malformed.
fn parse_blocks(toks: &[Token]) -> Option<Vec<Block>> {
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        let is_sp = matches!(&toks[i].tok, Tok::Word(w) if w.eq_ignore_ascii_case("SearchPaths"));
        if !(is_sp && toks.get(i + 1).is_some_and(|t| t.tok == Tok::Open)) {
            i += 1;
            continue;
        }
        let mut depth = 1;
        let mut j = i + 2;
        // line -> (words at depth 1, whether any brace sits on that line)
        let mut lines: Vec<(usize, Vec<&str>, bool)> = Vec::new();
        while j < toks.len() && depth > 0 {
            let t = &toks[j];
            let entry = match lines.last_mut() {
                Some(l) if l.0 == t.line => l,
                _ => {
                    lines.push((t.line, Vec::new(), false));
                    lines.last_mut().unwrap()
                }
            };
            match &t.tok {
                Tok::Open => {
                    depth += 1;
                    entry.2 = true;
                }
                Tok::Close => {
                    depth -= 1;
                    entry.2 = true;
                }
                Tok::Word(w) if depth == 1 => entry.1.push(w),
                Tok::Word(_) => {}
            }
            j += 1;
        }
        if depth != 0 {
            return None; // never closed
        }
        let mut block = Block { has_addons: false, anchor: None };
        for (line, words, braces) in &lines {
            if words.len() >= 2 && words[0].eq_ignore_ascii_case("Game") {
                if same_path(words[1], ADDONS) {
                    block.has_addons = true;
                }
                if block.anchor.is_none() && words.len() == 2 && !braces && words[1].eq_ignore_ascii_case("citadel") {
                    block.anchor = Some(*line);
                }
            }
        }
        blocks.push(block);
        i = j;
    }
    Some(blocks)
}

pub fn ensure_addons_path(text: &str) -> PatchResult {
    let Some(toks) = lex(text) else { return PatchResult::Unrecognized };
    let Some(blocks) = parse_blocks(&toks) else { return PatchResult::Unrecognized };
    if blocks.is_empty() {
        return PatchResult::Unrecognized;
    }
    if blocks.iter().any(|b| b.has_addons) {
        return PatchResult::AlreadyPresent;
    }
    let Some(anchor) = blocks.iter().find_map(|b| b.anchor) else { return PatchResult::Unrecognized };

    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let Some(anchor_line) = lines.get(anchor) else { return PatchResult::Unrecognized };

    let indent: String = anchor_line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    let eol = if anchor_line.ends_with("\r\n") {
        "\r\n"
    } else if anchor_line.ends_with('\n') || !text.contains("\r\n") {
        "\n"
    } else {
        "\r\n" // last line without a terminator: follow the rest of the file
    };
    let new_line = format!("{indent}Game                {ADDONS}{eol}");

    let mut out = String::with_capacity(text.len() + new_line.len());
    for (i, l) in lines.iter().enumerate() {
        if i == anchor {
            out.push_str(&new_line);
        }
        out.push_str(l);
    }
    PatchResult::Patched(out)
}

/// Replace `path` with `data` so that a crash can never leave a half-written file: write a sibling
/// temp file, flush it to disk, then rename it over the original.
pub fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".ddmm-tmp");
    let tmp = std::path::PathBuf::from(tmp);
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        drop(f);
        // Keep the original's permissions (e.g. a read-only flag would be a user choice to respect).
        if let Ok(meta) = std::fs::metadata(path) {
            let _ = std::fs::set_permissions(&tmp, meta.permissions());
        }
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "GameInfo\r\n{\r\n\tSearchPaths\r\n\t{\r\n\t\tGame_LowViolence    citadel_lv\r\n\t\tMod                 citadel\r\n\t\tWrite               citadel\r\n\t\tGame                citadel\r\n\t\tGame                core\r\n\t}\r\n}\r\n";

    #[test]
    fn inserts_before_game_citadel_keeping_crlf() {
        let PatchResult::Patched(s) = ensure_addons_path(BASE) else { panic!() };
        let lines: Vec<&str> = s.lines().collect();
        let i = lines.iter().position(|l| l.contains("citadel/addons")).unwrap();
        assert!(lines[i + 1].trim_start().starts_with("Game") && lines[i + 1].trim_end().ends_with("citadel"));
        assert!(s.contains("citadel/addons\r\n"));
        assert_eq!(s.lines().count(), BASE.lines().count() + 1);
    }

    #[test]
    fn idempotent() {
        let PatchResult::Patched(s) = ensure_addons_path(BASE) else { panic!() };
        assert_eq!(ensure_addons_path(&s), PatchResult::AlreadyPresent);
    }

    #[test]
    fn respects_foreign_manager_edits() {
        // Shape taken from a real machine patched by other managers.
        let t = "SearchPaths\n{\n Game citadel/deadworks_mods\n addonroot citadel/deadworks_addons\n Game citadel/addons\n Mod citadel\n Game citadel\n}\n";
        assert_eq!(ensure_addons_path(t), PatchResult::AlreadyPresent);
    }

    #[test]
    fn refuses_unknown_layout() {
        assert_eq!(ensure_addons_path("nothing here"), PatchResult::Unrecognized);
        assert_eq!(ensure_addons_path("SearchPaths\n{\n Mod core\n}\n"), PatchResult::Unrecognized);
    }

    #[test]
    fn quoted_values_are_understood() {
        let present = "SearchPaths\n{\n Game \"citadel/addons\"\n Game \"citadel\"\n}\n";
        assert_eq!(ensure_addons_path(present), PatchResult::AlreadyPresent);
        let missing = "SearchPaths\n{\n\t\"Game\"\t\"citadel\"\n}\n";
        let PatchResult::Patched(s) = ensure_addons_path(missing) else { panic!() };
        assert!(s.starts_with("SearchPaths\n{\n\tGame                citadel/addons\n\t\"Game\"\t\"citadel\"\n"));
    }

    #[test]
    fn backslash_and_trailing_slash_count_as_present() {
        assert_eq!(ensure_addons_path("SearchPaths\n{\n Game citadel\\addons\\\n Game citadel\n}\n"), PatchResult::AlreadyPresent);
    }

    #[test]
    fn commented_out_entries_are_ignored() {
        let t = "SearchPaths\n{\n // Game citadel/addons\n /* Game citadel/addons\n Game citadel/addons */\n Game citadel\n}\n";
        let PatchResult::Patched(s) = ensure_addons_path(t) else { panic!("should patch") };
        // Three mentions sit inside comments; exactly one real line was added.
        assert_eq!(s.matches("citadel/addons").count(), 4);
        assert!(s.contains(" Game                citadel/addons\n Game citadel\n}"));
        assert_eq!(ensure_addons_path(&s), PatchResult::AlreadyPresent);
    }

    #[test]
    fn braces_inside_block_comments_do_not_confuse_depth() {
        let t = "SearchPaths\n{\n /* } */\n Game citadel\n}\n";
        assert!(matches!(ensure_addons_path(t), PatchResult::Patched(_)));
    }

    #[test]
    fn braces_on_the_same_line_as_entries() {
        let t = "SearchPaths { Game citadel/addons Game citadel }\n";
        assert_eq!(ensure_addons_path(t), PatchResult::AlreadyPresent);
        // An anchor sharing its line with other tokens cannot be patched line-wise: refuse.
        assert_eq!(ensure_addons_path("SearchPaths { Game citadel }\n"), PatchResult::Unrecognized);
    }

    #[test]
    fn multiple_blocks_and_nested_blocks() {
        let t = "Other\n{\n SearchPaths\n {\n Mod core\n }\n}\nSearchPaths\n{\n Sub\n {\n Game citadel\n }\n Game citadel\n}\n";
        let PatchResult::Patched(s) = ensure_addons_path(t) else { panic!() };
        // Inserted next to the depth-1 anchor of the second block, not the nested one.
        assert!(s.contains(" Game                citadel/addons\n Game citadel\n}\n"));
        assert_eq!(s.matches("citadel/addons").count(), 1);
    }

    #[test]
    fn addons_in_any_block_counts() {
        let t = "SearchPaths\n{\n Game citadel\n}\nSearchPaths\n{\n Game citadel/addons\n}\n";
        assert_eq!(ensure_addons_path(t), PatchResult::AlreadyPresent);
    }

    #[test]
    fn malformed_input_is_refused() {
        assert_eq!(ensure_addons_path("SearchPaths\n{\n Game citadel\n"), PatchResult::Unrecognized); // never closed
        assert_eq!(ensure_addons_path("SearchPaths\n{\n /* Game citadel\n}\n"), PatchResult::Unrecognized); // open comment
        assert_eq!(ensure_addons_path("SearchPaths\n{\n Game \"citadel\n}\n"), PatchResult::Unrecognized); // open string
    }

    #[test]
    fn mixed_line_endings_and_missing_final_newline() {
        // The inserted line copies the line ending of the anchor line it precedes.
        let crlf_anchor = "SearchPaths\n{\n Mod core\n Game citadel\r\n}";
        let PatchResult::Patched(s) = ensure_addons_path(crlf_anchor) else { panic!() };
        assert!(s.contains(" Game                citadel/addons\r\n Game citadel\r\n}"));
        let lf_anchor = "SearchPaths\r\n{\r\n Game citadel\n}\r\n";
        let PatchResult::Patched(s) = ensure_addons_path(lf_anchor) else { panic!() };
        assert!(s.contains(" Game                citadel/addons\n Game citadel\n}"));
    }

    #[test]
    fn non_ascii_text_is_preserved() {
        let t = "// Заметка\nSearchPaths\n{\n Game citadel\n}\n";
        let PatchResult::Patched(s) = ensure_addons_path(t) else { panic!() };
        assert!(s.starts_with("// Заметка\n"));
    }

    #[test]
    fn write_atomic_replaces_and_cleans_up() {
        let dir = std::env::temp_dir().join(format!("ddmm-gi-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("gameinfo.gi");
        std::fs::write(&p, "old").unwrap();
        write_atomic(&p, b"new").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "new");
        assert!(!dir.join("gameinfo.gi.ddmm-tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
