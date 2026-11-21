//! Saves a screenshot or a short recording of the 3D preview to a place the player picked in the save dialog.
use tauri::ipc::{InvokeBody, Request};

/// The body is the raw file; the path comes in the `path` header (percent-encoded) from the save dialog.
/// Only png and webm files are written, so a stray call cannot drop other file types.
#[tauri::command]
pub fn save_capture(req: Request<'_>) -> Result<(), String> {
    let InvokeBody::Raw(bytes) = req.body() else { return Err("bad_body".into()) };
    let raw = req.headers().get("path").and_then(|v| v.to_str().ok()).ok_or("no_path")?;
    let path = percent_decode(raw);
    let lower = path.to_lowercase();
    if !(lower.ends_with(".png") || lower.ends_with(".webm")) {
        return Err("bad_type".into());
    }
    if bytes.len() > 200 * 1024 * 1024 {
        return Err("too_big".into());
    }
    std::fs::write(&path, bytes).map_err(|e| e.to_string())
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
