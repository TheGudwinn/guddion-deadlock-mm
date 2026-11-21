//! The four signature abilities of a hero for the 3D preview: name and icon from the public asset API,
//! and the game's animation that plays when the ability is cast.
use super::library::game;
use super::AppState;
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};
use tauri::State;

const HEROES_URL: &str = "https://api.deadlock-api.com/v1/assets/heroes";
const ITEMS_URL: &str = "https://api.deadlock-api.com/v1/assets/items";

#[derive(Serialize, Clone)]
pub struct AbilitySlot {
    pub class: String,
    pub name: String,
    pub icon: Option<String>,
    /// File stem of the animation, when the game has one for this ability.
    pub clip: Option<String>,
}

#[derive(Clone)]
struct Meta {
    class: String,
    name: String,
    icon: Option<String>,
}

type MetaMap = HashMap<String, Vec<Meta>>;

fn cache() -> &'static tokio::sync::Mutex<HashMap<String, Arc<MetaMap>>> {
    static C: OnceLock<tokio::sync::Mutex<HashMap<String, Arc<MetaMap>>>> = OnceLock::new();
    C.get_or_init(|| tokio::sync::Mutex::new(HashMap::new()))
}

/// Hero code -> its signature abilities, fetched once per language and run. The item list is large (several MB),
/// so only the abilities of heroes are kept.
async fn load_meta(client: reqwest::Client, lang: &str) -> Result<Arc<MetaMap>, String> {
    let mut guard = cache().lock().await;
    if let Some(m) = guard.get(lang) {
        return Ok(m.clone());
    }
    let get = |url: String| {
        let client = client.clone();
        async move {
            client
                .get(url)
                .timeout(std::time::Duration::from_secs(40))
                .send()
                .await
                .map_err(|e| e.to_string())?
                .error_for_status()
                .map_err(|e| e.to_string())?
                .json::<serde_json::Value>()
                .await
                .map_err(|e| e.to_string())
        }
    };
    let heroes = get(HEROES_URL.to_string()).await?;
    let items = get(format!("{ITEMS_URL}?language={lang}")).await?;
    let mut info: HashMap<String, (String, Option<String>)> = HashMap::new();
    for it in items.as_array().into_iter().flatten() {
        if it.get("type").and_then(|v| v.as_str()) != Some("ability") {
            continue;
        }
        let (Some(c), Some(n)) = (it.get("class_name").and_then(|v| v.as_str()), it.get("name").and_then(|v| v.as_str())) else { continue };
        let icon = it.get("image_webp").or_else(|| it.get("image")).and_then(|v| v.as_str()).map(str::to_string);
        info.entry(c.to_string()).or_insert((n.to_string(), icon));
    }
    let mut out = MetaMap::new();
    for h in heroes.as_array().into_iter().flatten() {
        let Some(code) = h.get("class_name").and_then(|v| v.as_str()).and_then(|c| c.strip_prefix("hero_")) else { continue };
        let slots = (1..=4)
            .filter_map(|i| {
                let class = h.pointer(&format!("/items/signature{i}"))?.as_str()?.to_string();
                let (name, icon) = info.get(&class).cloned().unwrap_or((class.clone(), None));
                Some(Meta { class, name, icon })
            })
            .collect();
        out.insert(code.to_string(), slots);
    }
    let out = Arc::new(out);
    guard.insert(lang.to_string(), out.clone());
    Ok(out)
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect()
}

fn collect_strings(v: &serde_json::Value, out: &mut Vec<String>) {
    match v {
        serde_json::Value::String(s) => out.push(norm(s)),
        serde_json::Value::Array(a) => a.iter().for_each(|x| collect_strings(x, out)),
        serde_json::Value::Object(o) => {
            for (k, x) in o {
                out.push(norm(k));
                collect_strings(x, out);
            }
        }
        _ => {}
    }
}

/// Words that name a phase or a pose of an ability animation rather than the ability itself.
const PHASE: &[&str] = &["intro", "loop", "fire", "start", "end", "release", "hold", "draw", "outro", "recover", "idle", "in", "air", "inair", "ground", "on", "grab", "lift", "throw", "missile", "channel", "up", "down", "walk", "leap", "item", "success", "copy"];

/// "ability_<name>_<phase>" clips grouped by ability name; each group knows the clip that best starts it.
fn clip_groups(clips: &[String]) -> Vec<(String, String)> {
    let mut groups: Vec<(String, Vec<&String>)> = Vec::new();
    for n in clips.iter().filter(|n| n.starts_with("ability_")) {
        let mut parts: Vec<&str> = n[8..].split('_').collect();
        while parts.len() > 1 && PHASE.contains(parts.last().unwrap()) {
            parts.pop();
        }
        let key = parts.join("_");
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, l)) => l.push(n),
            None => groups.push((key, vec![n])),
        }
    }
    const MAIN: &[&str] = &["fire", "release", "start", "intro", "hit", "throw", "cast", "slam"];
    groups
        .into_iter()
        .map(|(k, l)| {
            let main = l.iter().find(|n| MAIN.iter().any(|m| n.contains(m))).unwrap_or(&l[0]);
            (k, (*main).clone())
        })
        .collect()
}

/// The animation of each ability: a group whose name appears in the ability's own data (sounds, particles,
/// class name); otherwise the hero's generic cast.
fn match_clips(abilities: &[(String, Vec<String>)], clips: &[String]) -> Vec<Option<String>> {
    let groups = clip_groups(clips);
    let cast = clips.iter().find(|n| n.ends_with("cast_start")).or_else(|| clips.iter().find(|n| n.ends_with("cast_loop"))).cloned();
    // the whole name of a group, or its first word (groundstrike for groundstrike_airdrop)
    let keys = |key: &str| -> Vec<String> {
        let first = key.split('_').next().unwrap_or(key);
        [norm(key), norm(first)].into_iter().filter(|k| k.len() >= 4).collect()
    };
    let mut taken: HashSet<usize> = HashSet::new();
    let mut found: Vec<Option<usize>> = vec![None; abilities.len()];
    // First the ability's own name, which is certain; then what its sounds and particles mention, among the
    // groups nobody has claimed (a trap that shares a sound with another ability must not take its animation).
    for by_name in [true, false] {
        for (n, (class, strings)) in abilities.iter().enumerate() {
            if found[n].is_some() {
                continue;
            }
            let class = norm(class);
            let best = groups
                .iter()
                .enumerate()
                .filter(|(i, _)| !taken.contains(i))
                // variants of an animation that another ability already owns (charged_shot_walk_*) are not free either
                .filter(|(_, (key, _))| by_name || !taken.iter().any(|t| groups[*t].0.split('_').next() == key.split('_').next()))
                .filter_map(|(i, (key, _))| {
                    keys(key)
                        .into_iter()
                        .filter(|k| if by_name { class.contains(k.as_str()) } else { strings.iter().any(|s| s.contains(k.as_str())) })
                        .map(|k| k.len())
                        .max()
                        .map(|len| (i, len))
                })
                .max_by_key(|(i, len)| (*len, std::cmp::Reverse(*i)));
            if let Some((i, _)) = best {
                taken.insert(i);
                found[n] = Some(i);
            }
        }
    }
    found.into_iter().map(|f| f.map(|i| groups[i].1.clone()).or_else(|| cast.clone())).collect()
}

/// The four signature abilities of a hero, in order, with their names (in `lang`: russian or english),
/// icons and the animation that shows each one.
#[tauri::command]
pub async fn hero_abilities(state: State<'_, AppState>, hero: String, lang: String) -> Result<Vec<AbilitySlot>, String> {
    let code = hero.trim_start_matches("hero_").to_ascii_lowercase();
    if code.is_empty() || !code.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("bad_hero".into());
    }
    let lang = if lang == "russian" { "russian" } else { "english" };
    let meta = load_meta(state.net.client(), lang).await?;
    let list = meta.get(&code).cloned().unwrap_or_default();
    let g = game(&state)?;
    tokio::task::spawn_blocking(move || -> Result<Vec<AbilitySlot>, String> {
        use source2_model::vpk_extract::VpkArchive;
        let base = VpkArchive::open(&g.citadel_dir.join("pak01_dir.vpk")).map_err(|e| e.to_string())?;
        let model = super::hero::base_model(&base, &code).ok_or("no_base_model")?;
        let listing = base.list_entries();
        let clips: Vec<String> = source2_model::nm_anim::model_clip_paths(&base, &model, &listing)
            .iter()
            .filter_map(|p| p.rsplit('/').next())
            .filter_map(|f| f.strip_suffix(".vnmclip_c"))
            .map(str::to_string)
            .collect();
        // the ability data of the game: what each ability plays and sounds like
        let data = (|| {
            let bytes = base.extract_entry("scripts/abilities.vdata_c").ok()?;
            let res = source2_model::Resource::parse(bytes).ok()?;
            source2_model::kv3::parse(res.block_bytes("DATA")?).ok()
        })();
        let abilities: Vec<(String, Vec<String>)> = list
            .iter()
            .map(|m| {
                let mut strings = Vec::new();
                if let Some(v) = data.as_ref().and_then(|d| d.get(&m.class)) {
                    collect_strings(&v.to_json(), &mut strings);
                }
                (m.class.clone(), strings)
            })
            .collect();
        let found = match_clips(&abilities, &clips);
        Ok(list.into_iter().zip(found).map(|(m, clip)| AbilitySlot { class: m.class, name: m.name, icon: m.icon, clip }).collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore]
    fn matching() {
        use source2_model::vpk_extract::VpkArchive;
        let citadel = std::path::Path::new("D:/SteamLibrary/steamapps/common/Deadlock/game/citadel");
        let base = VpkArchive::open(&citadel.join("pak01_dir.vpk")).unwrap();
        let listing = base.list_entries();
        let res = source2_model::Resource::parse(base.extract_entry("scripts/abilities.vdata_c").unwrap()).unwrap();
        let data = source2_model::kv3::parse(res.block_bytes("DATA").unwrap()).unwrap();
        let cases: [(&str, [&str; 4]); 3] = [
            ("lash", ["citadel_ability_lash_down_strike", "citadel_ability_lash", "ability_lash_flog", "citadel_ability_lash_ultimate"]),
            ("orion", ["ability_charged_shot", "ability_power_jump", "ability_immobilize_trap", "ability_guided_arrow"]),
            ("tengu", ["citadel_ability_tengu_urn", "citadel_ability_tangotether", "citadel_ability_tengu_stone_form", "citadel_ability_tengu_airlift"]),
        ];
        for (code, classes) in cases {
            let model = super::super::hero::base_model(&base, code).unwrap();
            let clips: Vec<String> = source2_model::nm_anim::model_clip_paths(&base, &model, &listing).iter().filter_map(|p| p.rsplit('/').next()).filter_map(|f| f.strip_suffix(".vnmclip_c")).map(str::to_string).collect();
            let abilities: Vec<(String, Vec<String>)> = classes
                .iter()
                .map(|c| {
                    let mut s = Vec::new();
                    if let Some(v) = data.get(c) {
                        super::collect_strings(&v.to_json(), &mut s);
                    }
                    (c.to_string(), s)
                })
                .collect();
            println!("MATCH {code} {:?}", classes.iter().zip(super::match_clips(&abilities, &clips)).collect::<Vec<_>>());
        }
    }
}
