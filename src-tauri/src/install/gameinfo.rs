//! Minimal, surgical edit of `gameinfo.gi`: make sure `Game citadel/addons` is listed in
//! SearchPaths before the `Game citadel` line. Nothing else in the file is touched, so edits made
//! by other managers (deadworks, Deadlock Mod Manager) are preserved.

#[derive(Debug, PartialEq, Eq)]
pub enum PatchResult {
    AlreadyPresent,
    Patched(String),
    /// SearchPaths block or the `Game citadel` anchor was not found; do not edit.
    Unrecognized,
}

const ADDONS: &str = "citadel/addons";

fn tokens(line: &str) -> Vec<&str> {
    line.split("//").next().unwrap_or("").split_whitespace().collect()
}

pub fn ensure_addons_path(text: &str) -> PatchResult {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let Some(sp) = lines.iter().position(|l| tokens(l).first().is_some_and(|t| t.eq_ignore_ascii_case("SearchPaths"))) else {
        return PatchResult::Unrecognized;
    };

    let mut depth = 0i32;
    let mut started = false;
    let mut anchor: Option<usize> = None;
    for (i, l) in lines.iter().enumerate().skip(sp + 1) {
        let t = tokens(l);
        if t.first() == Some(&"{") {
            depth += 1;
            started = true;
            continue;
        }
        if t.first() == Some(&"}") {
            depth -= 1;
            if started && depth <= 0 {
                break;
            }
            continue;
        }
        if !started {
            continue;
        }
        if t.len() >= 2 && t[0].eq_ignore_ascii_case("Game") {
            if t[1].eq_ignore_ascii_case(ADDONS) {
                return PatchResult::AlreadyPresent;
            }
            if anchor.is_none() && t[1].eq_ignore_ascii_case("citadel") {
                anchor = Some(i);
            }
        }
    }
    let Some(a) = anchor else { return PatchResult::Unrecognized };

    let anchor_line = lines[a];
    let indent: String = anchor_line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
    let eol = if anchor_line.ends_with("\r\n") { "\r\n" } else { "\n" };
    let new_line = format!("{indent}Game                {ADDONS}{eol}");

    let mut out = String::with_capacity(text.len() + new_line.len());
    for (i, l) in lines.iter().enumerate() {
        if i == a {
            out.push_str(&new_line);
        }
        out.push_str(l);
    }
    PatchResult::Patched(out)
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
}
