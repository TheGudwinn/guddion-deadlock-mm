//! Launch options and performance presets, stored as `game_prefs.json` in the app data folder.
//!
//! Launch options are passed to Steam (`steam://run/<id>//<args>`). Performance console variables go
//! into a clearly marked block of `autoexec.cfg`; text outside that block is never touched.
use crate::install::gameinfo::write_atomic;
use crate::steam::locate::DEADLOCK_APP_ID;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const BLOCK_START: &str = "// >>> Guddion performance (managed block, edit it in the app)";
pub const BLOCK_END: &str = "// <<< Guddion performance";

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GfxApi {
    #[default]
    Default,
    Dx11,
    Vulkan,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WindowMode {
    #[default]
    Default,
    Windowed,
    Fullscreen,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VSync {
    #[default]
    Default,
    On,
    Off,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct GamePrefs {
    pub novid: bool,
    pub console: bool,
    pub high_priority: bool,
    pub api: GfxApi,
    pub window: WindowMode,
    /// Extra launch options typed by the user.
    pub extra: String,
    /// 0 = do not set.
    pub fps_max: u32,
    pub vsync: VSync,
    pub show_fps: bool,
    pub no_focus_sleep: bool,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PrefsError {
    #[error("bad launch option: {0}")]
    BadOption(String),
    #[error("too many launch options")]
    TooMany,
    #[error("fps limit must be between 30 and 1000")]
    BadFps,
}

impl GamePrefs {
    pub fn validate(&self) -> Result<(), PrefsError> {
        self.extra_tokens()?;
        if self.fps_max != 0 && !(30..=1000).contains(&self.fps_max) {
            return Err(PrefsError::BadFps);
        }
        Ok(())
    }

    /// Words of the free-form field. Only plain option characters are allowed so nothing typed here can
    /// become part of a command line or a URL.
    fn extra_tokens(&self) -> Result<Vec<String>, PrefsError> {
        let tokens: Vec<String> = self.extra.split_whitespace().map(str::to_string).collect();
        if tokens.len() > 24 {
            return Err(PrefsError::TooMany);
        }
        for t in &tokens {
            let ok = t.len() <= 64 && t.chars().all(|c| c.is_ascii_alphanumeric() || "-+_.=".contains(c));
            if !ok {
                return Err(PrefsError::BadOption(t.clone()));
            }
        }
        Ok(tokens)
    }

    pub fn launch_args(&self) -> Result<Vec<String>, PrefsError> {
        let mut a: Vec<String> = Vec::new();
        if self.novid {
            a.push("-novid".into());
        }
        if self.console {
            a.push("-console".into());
        }
        if self.high_priority {
            a.push("-high".into());
        }
        match self.api {
            GfxApi::Default => {}
            GfxApi::Dx11 => a.push("-dx11".into()),
            GfxApi::Vulkan => a.push("-vulkan".into()),
        }
        match self.window {
            WindowMode::Default => {}
            WindowMode::Windowed => a.push("-windowed".into()),
            WindowMode::Fullscreen => a.push("-fullscreen".into()),
        }
        for t in self.extra_tokens()? {
            if !a.contains(&t) {
                a.push(t);
            }
        }
        Ok(a)
    }

    /// The text of the managed `autoexec.cfg` block, or `None` when nothing is set.
    pub fn autoexec_block(&self) -> Option<String> {
        let mut lines = Vec::new();
        if self.fps_max != 0 {
            lines.push(format!("fps_max {}", self.fps_max));
        }
        match self.vsync {
            VSync::Default => {}
            VSync::On => lines.push("mat_vsync 1".into()),
            VSync::Off => lines.push("mat_vsync 0".into()),
        }
        if self.show_fps {
            lines.push("cl_showfps 1".into());
        }
        if self.no_focus_sleep {
            lines.push("engine_no_focus_sleep 0".into());
        }
        if lines.is_empty() {
            return None;
        }
        Some(format!("{BLOCK_START}\n{}\n{BLOCK_END}\n", lines.join("\n")))
    }
}

/// `steam://` address that starts Deadlock with the given options.
pub fn steam_url(args: &[String]) -> String {
    if args.is_empty() {
        format!("steam://rungameid/{DEADLOCK_APP_ID}")
    } else {
        format!("steam://run/{DEADLOCK_APP_ID}//{}", args.join("%20"))
    }
}

/// `existing` with the managed block replaced by `block` (removed when `None`). Everything outside the
/// block is kept, and the file's line endings are kept.
pub fn apply_block(existing: &str, block: Option<&str>) -> String {
    let crlf = existing.contains("\r\n");
    let text = existing.replace("\r\n", "\n");
    let (head, tail) = match (text.find(BLOCK_START), text.find(BLOCK_END)) {
        (Some(s), Some(e)) if e > s => {
            let after = e + BLOCK_END.len();
            let after = text[after..].find('\n').map(|n| after + n + 1).unwrap_or(text.len());
            (text[..s].to_string(), text[after..].to_string())
        }
        _ => (text.clone(), String::new()),
    };
    let mut out = head;
    if let Some(b) = block {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(b);
    }
    out.push_str(&tail);
    if crlf { out.replace('\n', "\r\n") } else { out }
}

pub fn load(path: &Path) -> GamePrefs {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save(path: &Path, p: &GamePrefs) -> std::io::Result<()> {
    write_atomic(path, &serde_json::to_vec_pretty(p).map_err(std::io::Error::other)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_and_url() {
        let p = GamePrefs { novid: true, console: true, api: GfxApi::Dx11, extra: "-novid +fps_max 144".into(), ..Default::default() };
        let a = p.launch_args().unwrap();
        assert_eq!(a, ["-novid", "-console", "-dx11", "+fps_max", "144"]);
        assert_eq!(steam_url(&a), "steam://run/1422450//-novid%20-console%20-dx11%20+fps_max%20144");
        assert_eq!(steam_url(&[]), "steam://rungameid/1422450");
    }

    #[test]
    fn rejects_dangerous_extra() {
        for bad in ["-a&calc", "x\"y", "a;b", "%20", "/c", "a|b", "$(x)"] {
            let p = GamePrefs { extra: bad.into(), ..Default::default() };
            assert!(p.launch_args().is_err(), "{bad}");
        }
    }

    #[test]
    fn fps_range() {
        assert!(GamePrefs { fps_max: 10, ..Default::default() }.validate().is_err());
        assert!(GamePrefs { fps_max: 144, ..Default::default() }.validate().is_ok());
    }

    #[test]
    fn block_is_replaced_and_removed_keeping_the_rest() {
        let p = GamePrefs { fps_max: 144, vsync: VSync::Off, ..Default::default() };
        let b = p.autoexec_block().unwrap();
        let first = apply_block("bind x jump\n", Some(&b));
        assert!(first.starts_with("bind x jump\n") && first.contains("fps_max 144"));
        let p2 = GamePrefs { fps_max: 60, ..Default::default() };
        let second = apply_block(&first, p2.autoexec_block().as_deref());
        assert!(second.contains("fps_max 60") && !second.contains("144") && !second.contains("mat_vsync"));
        assert_eq!(apply_block(&second, None), "bind x jump\n");
    }

    #[test]
    fn block_keeps_crlf_and_text_after_it() {
        let b = GamePrefs { show_fps: true, ..Default::default() }.autoexec_block().unwrap();
        let text = apply_block("a 1\r\nb 2\r\n", Some(&b));
        assert!(text.contains("\r\n") && !text.replace("\r\n", "").contains('\n'));
        let after = format!("{text}c 3\r\n");
        assert_eq!(apply_block(&after, None), "a 1\r\nb 2\r\nc 3\r\n");
    }
}
