use std::collections::{BTreeMap, BTreeSet};

use super::sequence::{
    clip_stem, coherent_phase_paths, family_prefix, is_additive_clip, is_in_air_variant,
};

pub const MAX_PREVIEW_SEQUENCE_CLIPS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewActionKind {
    Movement,
    Ability,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewAction {
    pub id: String,
    pub kind: PreviewActionKind,
    pub clips: Vec<String>,
    /// The ability name a clip group was found by, when its clip names spell
    /// one.
    pub ability: Option<String>,
}

const MOVEMENT: &[(&str, &[&[&str]])] = &[
    (
        "idle",
        &[
            &["out_of_combat_stand_idle"],
            &["weapon_stand_idle"],
            &["stand_idle"],
            &["idle"],
        ],
    ),
    ("heroSelect", &[&["ui_hero_select"], &["hero_select"]]),
    (
        "run",
        &[&["out_of_combat_run_n"], &["weapon_run_n"], &["run_n"]],
    ),
    ("weaponRun", &[&["weapon_run_n"]]),
    (
        "crouch",
        &[&["weapon_crouch_run_n"], &["out_of_combat_crouch_run_n"]],
    ),
    (
        "jump",
        &[&["jump_ground", "in_air_loop_apex", "in_air_loop_down"]],
    ),
    ("slide", &[&["slide_start", "slide_loop", "slide_getup"]]),
    ("dash", &[&["dash_ground"], &["dash_air_forward"]]),
    ("mantle", &[&["mantle_64"], &["mantle_96"], &["mantle_32"]]),
    ("melee", &[&["melee_quick_1", "melee_quick_2"]]),
    ("heavyMelee", &[&["melee_start", "melee_hit"]]),
    ("parry", &[&["parry", "parry_success"]]),
    (
        "reload",
        &[&["reload_idle_start", "reload_idle_round", "reload_idle_end"]],
    ),
    (
        "zipline",
        &[&["zipline_attached_start", "zipline_attached_loop"]],
    ),
    (
        "knockdown",
        &[&[
            "knockdown_large_start",
            "knockdown_large_loop",
            "knockdown_large_getup",
        ]],
    ),
];

fn squash(text: &str) -> String {
    text.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|ch| ch.to_ascii_lowercase())
        .collect()
}

/// A clip group named after an ability, and the ability it names: the one to
/// three tokens after any `ability_` prefix spell the whole name. That finds
/// Bebop's `hyperbeam_fire` for "Hyper Beam", and keeps Warden's
/// `ability_last_stand_idle` whole although `stand` reads as a stance.
fn named_family<'a>(path: &str, ability_names: &'a [String]) -> Option<(String, &'a str)> {
    let stem = clip_stem(path);
    let (prefix, name) = match stem.find("ability_") {
        Some(index) => ("ability_", &stem[index + "ability_".len()..]),
        None => ("", stem),
    };
    let tokens: Vec<_> = name.split('_').collect();
    // An unprefixed clip needs a phase after the name to read as an ability.
    let longest = tokens.len() - usize::from(prefix.is_empty());
    (1..=longest.min(3)).rev().find_map(|count| {
        let words = &tokens[..count];
        ability_names
            .iter()
            .find(|name| name.len() >= 4 && squash(name) == squash(&words.concat()))
            .map(|name| (format!("{prefix}{}", words.join("_")), name.as_str()))
    })
}

/// Select whole playable moves, excluding additive layers and stance fragments.
/// Ability phases share the resolver's ordering and directional policy.
/// `ability_names` (display names such as "Hyper Beam") find abilities whose
/// clips do not carry the `ability_` prefix.
pub fn preview_actions(
    paths: impl IntoIterator<Item = String>,
    ability_names: &[String],
) -> Vec<PreviewAction> {
    let paths: BTreeSet<_> = paths
        .into_iter()
        .filter(|path| {
            path.ends_with(".vnmclip_c") && !path.contains(".vnmclip+") && !is_additive_clip(path)
        })
        .collect();
    let find = |name: &str| {
        paths
            .iter()
            .find(|path| clip_stem(path) == name)
            .or_else(|| {
                paths.iter().find(|path| {
                    family_prefix(path).is_none() && clip_stem(path).ends_with(&format!("_{name}"))
                })
            })
            .cloned()
    };
    let mut actions: Vec<PreviewAction> = Vec::new();
    for &(id, alternatives) in MOVEMENT {
        for sequence in alternatives {
            if find(sequence[0]).is_none() {
                continue;
            }
            let clips: Vec<_> = sequence.iter().filter_map(|name| find(name)).collect();
            if !actions.iter().any(|action| action.clips == clips) {
                actions.push(PreviewAction {
                    id: id.to_string(),
                    kind: PreviewActionKind::Movement,
                    clips,
                    ability: None,
                });
            }
            break;
        }
    }
    let mut families: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut named: BTreeMap<String, &str> = BTreeMap::new();
    for path in paths {
        let family = match named_family(&path, ability_names) {
            Some((family, ability)) => {
                named.insert(family.clone(), ability);
                family
            }
            None => match family_prefix(&path) {
                Some(family) => family,
                None => continue,
            },
        };
        families.entry(family).or_default().push(path);
    }
    for (family, paths) in families {
        let Some(primary) = paths
            .iter()
            .find(|path| !is_in_air_variant(clip_stem(path)))
            .cloned()
        else {
            continue;
        };
        let clips: Vec<_> = coherent_phase_paths(&primary, &family, paths)
            .into_iter()
            .take(MAX_PREVIEW_SEQUENCE_CLIPS)
            .map(|(_, path)| path)
            .collect();
        if !clips.is_empty() {
            actions.push(PreviewAction {
                id: format!("ability:{}", family.trim_start_matches("ability_")),
                kind: PreviewActionKind::Ability,
                clips,
                ability: named.get(&family).map(|name| name.to_string()),
            });
        }
    }
    actions.truncate(512);
    actions
}

