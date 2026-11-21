//! 3D preview of heroes: the base game's model or the model a mod puts in its place.
use super::library::game;
use super::AppState;
use std::path::Path;
use std::sync::Arc;
use sha2::Digest;
use tauri::State;

const BINDINGS: &str = include_str!("../../assets/hero-model-bindings.json");

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// The model the game itself uses for a hero, from the table of hero code -> model.
fn base_models(code: &str) -> Vec<String> {
    let v: serde_json::Value = serde_json::from_str(BINDINGS).unwrap_or_default();
    v["bindings"][format!("hero_{code}")]
        .as_array()
        .map(|a| a.iter().filter_map(|p| p.as_str()).map(|p| format!("{p}_c")).collect())
        .unwrap_or_default()
}

/// The model the game's own hero table names (`scripts/heroes.vdata`), which follows game updates; the
/// bundled table is only the fallback.
fn game_models(base: &source2_model::vpk_extract::VpkArchive, code: &str) -> Vec<String> {
    let from_game = (|| {
        let bytes = base.extract_entry("scripts/heroes.vdata_c").ok()?;
        let res = source2_model::Resource::parse(bytes).ok()?;
        let data = source2_model::kv3::parse(res.block_bytes("DATA")?).ok()?;
        let path = data.get(&format!("hero_{code}"))?.get("m_strModelName")?.as_string()?.to_string();
        Some(format!("{}_c", path.to_ascii_lowercase()))
    })();
    match from_game {
        Some(p) => vec![p],
        None => base_models(code),
    }
}

/// The base game's model path of a hero.
pub fn base_model(base: &source2_model::vpk_extract::VpkArchive, code: &str) -> Option<String> {
    game_models(base, code).into_iter().next()
}

/// The main model of a hero inside one VPK.
fn pick_model(entries: &[String], codes: &[String], wanted: Vec<String>) -> Option<String> {
    let lower: Vec<String> = entries.iter().map(|e| e.to_ascii_lowercase()).collect();
    for w in &wanted {
        if let Some(i) = lower.iter().position(|e| e == w) {
            return Some(entries[i].clone());
        }
    }
    let stems: Vec<String> = wanted.iter().filter_map(|w| w.rsplit('/').next().map(str::to_string)).collect();
    let skip = ["arm", "gib", "ragdoll", "lod", "weapon", "viewmodel", "wip_"];
    let mut best: Option<&String> = None;
    for (i, e) in lower.iter().enumerate() {
        if !e.ends_with(".vmdl_c") || !e.starts_with("models/heroes") {
            continue;
        }
        let in_hero = codes.iter().any(|code| e.split('/').any(|s| s == code || s.starts_with(&format!("{code}_")) || s.starts_with(&format!("hero_{code}"))));
        if !in_hero {
            continue;
        }
        let file = e.rsplit('/').next().unwrap_or(e);
        if stems.iter().any(|s| s == file) {
            return Some(entries[i].clone());
        }
        if skip.iter().any(|s| file.contains(s)) {
            continue;
        }
        if best.map_or(true, |b| entries[i].len() < b.len()) {
            best = Some(&entries[i]);
        }
    }
    best.cloned()
}

/// Decodes one hero; see `hero_model`.
pub fn render(citadel: &Path, addons: &Path, hero: &str, codes: &[String], addon: Option<&str>) -> Result<Vec<u8>, String> {
    render_clip(citadel, addons, hero, codes, addon, None)
}

/// Like `render`, with the game's animation `clip` (a file stem) playing instead of the idle one.
pub fn render_clip(citadel: &Path, addons: &Path, hero: &str, codes: &[String], addon: Option<&str>, clip: Option<&str>) -> Result<Vec<u8>, String> {
        use source2_model::vpk_extract::VpkArchive;
        let code = hero.trim_start_matches("hero_").to_ascii_lowercase();
        if code.is_empty() || !code.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err("bad_hero".into());
        }
        let base = Arc::new(VpkArchive::open(&citadel.join("pak01_dir.vpk")).map_err(err)?);
        let wanted = game_models(&base, &code);
        let mut names: Vec<String> = codes.iter().map(|c| c.to_ascii_lowercase()).filter(|c| !c.is_empty() && c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_')).collect();
        names.push(code.clone());
        let own: VpkArchive;
        let archive: &VpkArchive = match &addon {
            None => &base,
            Some(name) => {
                if name.contains(['/', '\\']) || name.contains("..") {
                    return Err("bad_addon".into());
                }
                let mut path = addons.join(name);
                if !path.is_file() {
                    path = addons.join(format!("{name}.disabled"));
                }
                if !path.is_file() {
                    return Err("addon_missing".into());
                }
                own = VpkArchive::open(Path::new(&path)).map_err(err)?.with_fallbacks(vec![base.clone()]);
                &own
            }
        };
        let entries = archive.list_entries();
        let model = match pick_model(&entries, &names, wanted) {
            Some(m) => m,
            None if addon.is_some() => return Err("no_model".into()),
            None => return Err("no_base_model".into()),
        };
        let out = source2_model::decode_model_glb_from_archive(archive, &model).map_err(err)?;
        // stand the hero in the game's idle pose instead of the stiff bind pose; the plain model is the fallback
        let out = source2_model::pose_model_with_clip(out, &base, archive, &model, clip);
        Ok(out.glb)
}

/// The hero as a binary glTF. `addon` is the file name of a mod in the game's `addons` folder; without it
/// the base game's model is drawn. Textures a mod does not carry are taken from the base game.
#[tauri::command]
pub async fn hero_model(state: State<'_, AppState>, hero: String, codes: Option<Vec<String>>, addon: Option<String>, clip: Option<String>) -> Result<tauri::ipc::Response, String> {
    let g = game(&state)?;
    let cache = state.app_dir.join("hero-cache");
    tokio::task::spawn_blocking(move || -> Result<Vec<u8>, String> {
        let codes = codes.unwrap_or_default();
        // only plain clip names are accepted (they end up in a cache key and a path lookup)
        let clip = clip.filter(|c| !c.is_empty() && c.len() < 80 && c.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_'));
        // A decoded hero is kept on disk: the key changes when the game or the mod file changes.
        let stamp = |p: &Path| std::fs::metadata(p).ok().map(|m| format!("{}:{:?}", m.len(), m.modified().ok())).unwrap_or_default();
        let mut key = format!("v{CACHE_VERSION}|{hero}|{codes:?}|{clip:?}|{}", stamp(&g.citadel_dir.join("pak01_dir.vpk")));
        if let Some(name) = &addon {
            let on = g.addons_dir.join(name);
            let path = if on.is_file() { on } else { g.addons_dir.join(format!("{name}.disabled")) };
            key.push_str(&format!("|{name}|{}", stamp(&path)));
        }
        let file = cache.join(format!("{}.glb", hex::encode(&sha2::Sha256::digest(key.as_bytes())[..12])));
        if let Ok(bytes) = std::fs::read(&file) {
            return Ok(bytes);
        }
        let glb = render_clip(&g.citadel_dir, &g.addons_dir, &hero, &codes, addon.as_deref(), clip.as_deref())?;
        if std::fs::create_dir_all(&cache).is_ok() {
            let _ = std::fs::write(&file, &glb);
            prune(&cache);
        }
        Ok(glb)
    })
    .await
    .map_err(err)?
    .map(tauri::ipc::Response::new)
}

/// Animations that mean something to a person looking at a skin. Directional run variants, aim offsets and
/// additive layers are only building blocks of the game's animation graph.
fn playable(stem: &str) -> bool {
    const SKIP_START: &[&str] = &["aim_", "loco_", "corner_lean", "item_", "weapon_run", "weapon_crouch", "out_of_combat_run", "out_of_combat_crouch_run", "airdrop_", "zipline", "rope_", "mantle", "wall_", "turn_", "death_", "flinch", "knockdown_in_air", "kockdown"];
    const SKIP_END: &[&str] = &["_n", "_ne", "_nw", "_e", "_w", "_s", "_se", "_sw", "_center", "_c", "_copy"];
    !stem.contains('+') && !SKIP_START.iter().any(|p| stem.starts_with(p)) && !SKIP_END.iter().any(|p| stem.ends_with(p))
}

/// The clips of a hero's base-game model that are worth showing, by file stem. Mods replace models, not
/// animations, so every skin of a hero plays the same ones.
#[tauri::command]
pub async fn hero_clips(state: State<'_, AppState>, hero: String) -> Result<Vec<String>, String> {
    let g = game(&state)?;
    tokio::task::spawn_blocking(move || -> Result<Vec<String>, String> {
        use source2_model::vpk_extract::VpkArchive;
        let code = hero.trim_start_matches("hero_").to_ascii_lowercase();
        if code.is_empty() || !code.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err("bad_hero".into());
        }
        let base = VpkArchive::open(&g.citadel_dir.join("pak01_dir.vpk")).map_err(err)?;
        let model = game_models(&base, &code).into_iter().next().ok_or("no_base_model")?;
        let listing = base.list_entries();
        let mut out: Vec<String> = source2_model::nm_anim::model_clip_paths(&base, &model, &listing)
            .iter()
            .filter_map(|p| p.rsplit('/').next())
            .filter_map(|f| f.strip_suffix(".vnmclip_c"))
            .filter(|s| playable(s))
            .map(str::to_string)
            .collect();
        out.sort();
        out.dedup();
        Ok(out)
    })
    .await
    .map_err(err)?
}

/// Bumped when the decoder changes what it writes, so old cache files are not used.
const CACHE_VERSION: u32 = 5;
const CACHE_KEEP: usize = 40;

/// Keeps the newest cached models only.
fn prune(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files: Vec<_> = rd.flatten().filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path()))).collect();
    if files.len() <= CACHE_KEEP {
        return;
    }
    files.sort();
    for (_, path) in &files[..files.len() - CACHE_KEEP] {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    /// Needs the real game; writes the model to `D:/ref` so it can be opened in a viewer.
    #[test]
    #[ignore]
    fn real_hero() {
        let citadel = std::path::Path::new("D:/SteamLibrary/steamapps/common/Deadlock/game/citadel");
        let addons = citadel.join("addons");
        for h in ["tengu", "lash", "orion", "necro"] {
            let t0 = std::time::Instant::now();
            let glb = super::render(citadel, &addons, h, &[], None).unwrap();
            println!("{h}: {:?}", t0.elapsed());
            std::fs::write(format!("D:/ref/{h}.glb"), &glb).unwrap();
            println!("{h}: {} bytes", glb.len());
        }
    }

    #[test]
    #[ignore]
    fn dump_model() {
        let citadel = std::path::Path::new("D:/SteamLibrary/steamapps/common/Deadlock/game/citadel");
        let a = source2_model::vpk_extract::VpkArchive::open(&citadel.join("pak01_dir.vpk")).unwrap();
        let b = a.extract_entry("scripts/heroes.vdata_c").unwrap();
        let res = source2_model::Resource::parse(b).unwrap();
        let v = source2_model::kv3::parse(res.block_bytes("DATA").unwrap()).unwrap();
        std::fs::write("D:/ref/heroes_vdata.json", serde_json::to_string_pretty(&v.to_json()).unwrap()).unwrap();
    }

    #[test]
    #[ignore]
    fn survey() {
        use source2_model::vpk_extract::VpkArchive;
        use std::collections::BTreeMap;
        let dir = std::path::Path::new("D:/SteamLibrary/steamapps/common/Deadlock/game/citadel/addons");
        for a in crate::library::scan::scan_addons(dir).unwrap() {
            let Ok(files) = &a.entries else { continue };
            let content = crate::library::describe::describe(files);
            let heroes: Vec<String> = content.heroes.iter().map(|h| format!("{}:{}", h.code, h.files)).collect();
            if heroes.is_empty() { continue; }
            let Ok(arc) = VpkArchive::open(&a.path) else { continue };
            let mut formats: BTreeMap<String, usize> = BTreeMap::new();
            let mut failed = 0;
            for e in arc.list_entries().iter().filter(|e| e.ends_with(".vtex_c") && e.contains("models/")) {
                let Ok(bytes) = arc.extract_entry(e) else { continue };
                let Ok(res) = source2_model::Resource::parse(bytes) else { failed += 1; continue };
                match source2_model::VtexHeader::parse(&res) {
                    Ok(h) => {
                        let ok = h.decode(&res.data).is_ok();
                        *formats.entry(format!("{:?}{}", h.format, if ok { "" } else { "(FAIL)" })).or_default() += 1;
                    }
                    Err(_) => failed += 1,
                }
            }
            println!("SURVEY {} {:?} {:?} hdrfail={}", a.file_name, heroes, formats, failed);
        }
    }

    /// Lists every material of the mods for the heroes named in DIAG_HEROES and whether it got a colour texture.
    #[test]
    #[ignore]
    fn diag() {
        let citadel = std::path::Path::new("D:/SteamLibrary/steamapps/common/Deadlock/game/citadel");
        let addons = citadel.join("addons");
        let want = ["chrono", "mirage", "vampirebat", "familiar"];
        for a in crate::library::scan::scan_addons(&addons).unwrap() {
            let Ok(files) = &a.entries else { continue };
            let content = crate::library::describe::describe(files);
            for h in &content.model_heroes {
                if !want.iter().any(|w| h.code.starts_with(w)) { continue; }
                let hero = want.iter().find(|w| h.code.starts_with(*w)).unwrap();
                match super::render(citadel, &addons, hero, &[h.code.clone()], Some(&a.file_name)) {
                    Ok(glb) => {
                        let len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
                        let json: serde_json::Value = serde_json::from_slice(&glb[20..20 + len]).unwrap();
                        std::fs::write(format!("D:/ref/d_{}.glb", a.file_name), &glb).unwrap(); println!("DIAG {} {} {} KB", a.file_name, h.code, glb.len() / 1024);
                        for m in json["materials"].as_array().unwrap() {
                            let pbr = &m["pbrMetallicRoughness"];
                            println!("   mat {} tex={} factor={} extras={}", m["name"], pbr["baseColorTexture"], pbr["baseColorFactor"], m["extras"]);
                        }
                    }
                    Err(e) => println!("DIAG {} {} ERR {e}", a.file_name, h.code),
                }
            }
        }
    }

    /// Dumps the materials and textures of the Paradox mod whose names contain `pentagram` / `rune` / `headglass`.
    #[test]
    #[ignore]
    fn paradox() {
        use source2_model::vpk_extract::VpkArchive;
        let arc = VpkArchive::open(std::path::Path::new("D:/SteamLibrary/steamapps/common/Deadlock/game/citadel/addons/pak34_dir.vpk")).unwrap();
        for e in arc.list_entries() {
            let hit = ["pentagram", "rune_glow", "headglass", "horns"].iter().any(|k| e.contains(k));
            if !hit { continue; }
            let name = e.rsplit('/').next().unwrap().to_string();
            let res = source2_model::Resource::parse(arc.extract_entry(&e).unwrap()).unwrap();
            if e.ends_with(".vmat_c") {
                let v = source2_model::kv3::parse(res.block_bytes("DATA").unwrap()).unwrap();
                std::fs::write(format!("D:/ref/pd_{name}.json"), serde_json::to_string_pretty(&v.to_json()).unwrap()).unwrap();
            } else if e.ends_with(".vtex_c") {
                let h = source2_model::VtexHeader::parse(&res).unwrap();
                if let Ok(d) = h.decode(&res.data) {
                    std::fs::write(format!("D:/ref/pd_{name}.{}x{}.rgba", d.width, d.height), &d.rgba).unwrap();
                    println!("PD {name} {}x{}", d.width, d.height);
                }
            }
        }
    }

    #[test]
    #[ignore]
    fn dump_mods() {
        let citadel = std::path::Path::new("D:/SteamLibrary/steamapps/common/Deadlock/game/citadel");
        let addons = citadel.join("addons");
        for (hero, code, file) in [
            ("hornet", "hornet_v3", "pak04_dir.vpk"),
            ("necro", "necro", "pak06_dir.vpk"),
            ("vampirebat", "vampirebat", "pak42_dir.vpk"),
            ("mirage", "mirage_v2", "pak38_dir.vpk"),
            ("chrono", "chrono", "pak34_dir.vpk"),
            ("wraith", "wraith", "pak29_dir.vpk"),
            ("familiar", "familiar", "pak27_dir.vpk"),
        ] {
            let t0 = std::time::Instant::now();
            match super::render(citadel, &addons, hero, &[code.to_string()], Some(file)) {
                Ok(glb) => {
                    std::fs::write(format!("D:/ref/m_{hero}.glb"), &glb).unwrap();
                    println!("DUMP {hero} {file} {} KB {:?}", glb.len() / 1024, t0.elapsed());
                }
                Err(e) => println!("DUMP {hero} {file} ERR {e}"),
            }
        }
    }

    #[test]
    #[ignore]
    fn tex_header() {
        use source2_model::vpk_extract::VpkArchive;
        let dir = std::path::Path::new("D:/SteamLibrary/steamapps/common/Deadlock/game/citadel/addons");
        for (file, needle) in [("pak34_dir.vpk", "chrono_gun_v2_vmat_g_tcolor"), ("pak34_dir.vpk", "headglass_vmat_g_tcolor"), ("pak27_dir.vpk", "watcher_face_vmat_g_tcolor"), ("pak42_dir.vpk", "t_r2t1lucymd10011face01_d"), ("pak38_dir.vpk", "gun_color")] {
            let arc = VpkArchive::open(&dir.join(file)).unwrap();
            for e in arc.list_entries().iter().filter(|e| e.contains(needle) && e.ends_with(".vtex_c")).take(2) {
                let res = source2_model::Resource::parse(arc.extract_entry(e).unwrap()).unwrap();
                let h = source2_model::VtexHeader::parse(&res).unwrap();
                let mut unc = 0;
                for l in 0..h.num_mip_levels.max(1) as u32 {
                    unc += h.format.mip_byte_size((h.width as u32 >> l).max(1), (h.height as u32 >> l).max(1)).unwrap_or(0);
                }
                println!("TEX {file} {e}\n   v{} flags={:#x} {}x{} d{} {:?} mips={} extra={:?} data_off={} data_len={} sum_unc={unc}",
                    h.version, h.flags, h.width, h.height, h.depth, h.format, h.num_mip_levels, h.extra, h.data_offset, res.data.len(), );
            }
        }
    }

    #[test]
    #[ignore]
    fn clips() {
        use source2_model::vpk_extract::VpkArchive;
        let citadel = std::path::Path::new("D:/SteamLibrary/steamapps/common/Deadlock/game/citadel");
        let base = VpkArchive::open(&citadel.join("pak01_dir.vpk")).unwrap();
        for code in ["lash", "orion", "inferno", "tengu", "hornet", "nano", "necro", "vampirebat", "mirage", "wraith"] {
            let model = super::game_models(&base, code).remove(0);
            for clip in [Some("weapon_stand_idle"), Some("idle"), Some("stand_idle"), None] {
                match source2_model::nm_anim::resolve_nm_clip_from_archive(&base, &model, clip, None) {
                    Ok(Some(r)) => {
                        let a = source2_model::nm_anim::load_nm_animation_from_archive(&base, &r.clip_path).unwrap();
                        println!("CLIP {code} want={clip:?} -> {} frames={} dur={:.2}s fps={:.1}", r.clip_path, a.clip.frame_count, a.clip.duration_seconds, a.fps());
                    }
                    Ok(None) => println!("CLIP {code} want={clip:?} -> none"),
                    Err(e) => println!("CLIP {code} want={clip:?} ERR {e}"),
                }
            }
        }
    }

    #[test]
    #[ignore]
    fn clip_names() {
        use source2_model::vpk_extract::VpkArchive;
        let citadel = std::path::Path::new("D:/SteamLibrary/steamapps/common/Deadlock/game/citadel");
        let base = VpkArchive::open(&citadel.join("pak01_dir.vpk")).unwrap();
        let listing = base.list_entries();
        for code in ["lash", "tengu", "orion"] {
            let model = super::game_models(&base, code).remove(0);
            let clips = source2_model::nm_anim::model_clip_paths(&base, &model, &listing);
            let names: Vec<&str> = clips.iter().map(|c| c.rsplit('/').next().unwrap()).collect();
            println!("NAMES {code} {} clips: {}", names.len(), names.join(" "));
        }
    }

    #[test]
    #[ignore]
    fn clip_render() {
        let citadel = std::path::Path::new("D:/SteamLibrary/steamapps/common/Deadlock/game/citadel");
        let addons = citadel.join("addons");
        for (h, c) in [("lash", Some("ability_flog")), ("lash", None), ("orion", Some("ability_guided_arrow_fire")), ("orion", None), ("orion", Some("ability_generic_cast")), ("orion", Some("ability_power_jump_leap_up"))] {
            let t0 = std::time::Instant::now();
            let glb = super::render_clip(citadel, &addons, h, &[], None, c).unwrap();
            let len = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
            let json: serde_json::Value = serde_json::from_slice(&glb[20..20 + len]).unwrap();
            println!("CLIP {h} {c:?} {:?} {} KB anims={}", t0.elapsed(), glb.len() / 1024, json["animations"].as_array().map(|a| a.len()).unwrap_or(0));
        }
    }

    #[test]
    #[ignore]
    fn dump_abilities() {
        let citadel = std::path::Path::new("D:/SteamLibrary/steamapps/common/Deadlock/game/citadel");
        let a = source2_model::vpk_extract::VpkArchive::open(&citadel.join("pak01_dir.vpk")).unwrap();
        for e in a.list_entries().iter().filter(|e| e.contains("vdata_c") && e.starts_with("scripts/")) {
            println!("VDATA {e}");
        }
        let b = a.extract_entry("scripts/abilities.vdata_c").unwrap();
        let res = source2_model::Resource::parse(b).unwrap();
        let v = source2_model::kv3::parse(res.block_bytes("DATA").unwrap()).unwrap();
        std::fs::create_dir_all("D:/ref").unwrap();
        std::fs::write("D:/ref/abilities_vdata.json", serde_json::to_string_pretty(&v.to_json()).unwrap()).unwrap();
    }

    #[test]
    #[ignore]
    fn skel_cmp() {
        use source2_model::vpk_extract::VpkArchive;
        use std::collections::HashMap;
        let citadel = std::path::Path::new("D:/SteamLibrary/steamapps/common/Deadlock/game/citadel");
        let base = VpkArchive::open(&citadel.join("pak01_dir.vpk")).unwrap();
        let modp = VpkArchive::open(&citadel.join("guddion_compat/pak01_dir.vpk")).unwrap();
        let path = "models/heroes_wip/inferno/inferno.vmdl_c";
        let load = |a: &VpkArchive| {
            let res = source2_model::Resource::parse(a.extract_entry(path).unwrap()).unwrap();
            source2_model::skeleton::parse_model_skeleton(&res).unwrap().unwrap()
        };
        let (g, m) = (load(&base), load(&modp));
        println!("SK bones game={} mod={}", g.bones.len(), m.bones.len());
        let same_idx = g.bones.iter().zip(&m.bones).take_while(|(a, b)| a.name == b.name).count();
        let moved = g.bones.iter().enumerate().filter(|(i, b)| m.bones.get(*i).map(|x| x.name != b.name).unwrap_or(true)).count();
        println!("SK first_index_mismatch={same_idx} bones_at_other_index={moved} mod[{same_idx}]={:?} game[{same_idx}]={:?}", m.bones.get(same_idx).map(|b| &b.name), g.bones.get(same_idx).map(|b| &b.name));
        let gi: HashMap<&str, usize> = g.bones.iter().enumerate().map(|(i, b)| (b.name.as_str(), i)).collect();
        let mi: HashMap<&str, usize> = m.bones.iter().enumerate().map(|(i, b)| (b.name.as_str(), i)).collect();
        let only_g: Vec<&&str> = gi.keys().filter(|k| !mi.contains_key(**k)).collect();
        let only_m: Vec<&&str> = mi.keys().filter(|k| !gi.contains_key(**k)).collect();
        println!("SK only_game({})={:?}", only_g.len(), &only_g[..only_g.len().min(25)]);
        println!("SK only_mod({})={:?}", only_m.len(), &only_m[..only_m.len().min(25)]);
        let mut parent_diff = 0; let mut rot_diff = 0; let mut pos_diff = 0;
        for (name, &i) in &mi {
            let Some(&j) = gi.get(name) else { continue };
            let (a, b) = (&m.bones[i], &g.bones[j]);
            let pn = |s: &source2_model::Skeleton, b: &source2_model::Bone| if b.parent < 0 { "-".to_string() } else { s.bones[b.parent as usize].name.clone() };
            if pn(&m, a) != pn(&g, b) { parent_diff += 1; if parent_diff <= 5 { println!("SK parent {name}: mod={} game={}", pn(&m, a), pn(&g, b)); } }
            let dr: f32 = a.bind_rot.iter().zip(&b.bind_rot).map(|(x, y)| (x - y).abs()).sum();
            let dp: f32 = a.bind_pos.iter().zip(&b.bind_pos).map(|(x, y)| (x - y).abs()).sum();
            if dr > 0.05 { rot_diff += 1; if rot_diff <= 8 { println!("SK rot {name}: mod={:?} game={:?}", a.bind_rot, b.bind_rot); } }
            if dp > 0.5 { pos_diff += 1; if pos_diff <= 8 { println!("SK pos {name}: mod={:?} game={:?}", a.bind_pos, b.bind_pos); } }
        }
        println!("SK parent_diff={parent_diff} rot_diff={rot_diff} pos_diff={pos_diff}");
        println!("SK root mod={:?} game={:?}", (&m.bones[0].name, m.bones[0].bind_rot), (&g.bones[0].name, g.bones[0].bind_rot));
    }
}
