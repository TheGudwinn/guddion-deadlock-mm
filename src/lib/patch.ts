import type { AddonContent, InstalledMod, UpdateReport } from "./api";

export type Risk = "high" | "mid" | "low";

export type Suspect = {
  mod: InstalledMod;
  risk: Risk;
  /** Why the mod is on the list: what it replaces in the game. */
  kinds: string[];
  /** The author has already published a newer version. */
  update: boolean;
};

const ORDER: Record<Risk, number> = { high: 0, mid: 1, low: 2 };

/** Content kinds that a game patch breaks most often (rebuilt models, effects, scripts, maps). */
const FRAGILE = new Set(["models", "effects", "scripts", "maps"]);
/** Interface files follow the game's UI layout, which changes with patches too, but breaks less loudly. */
const MEDIUM = new Set(["interface"]);

const ackKey = "kea.patchAck";

export function patchAcked(build: string | null): boolean {
  try {
    return !!build && localStorage.getItem(ackKey) === build;
  } catch {
    return false;
  }
}

export function ackPatch(build: string | null) {
  try {
    if (build) localStorage.setItem(ackKey, build);
  } catch {
    /* storage can be unavailable; the list just shows again next time */
  }
}

/**
 * Enabled mods this app installed on an older game build than the current one, riskiest first.
 * The risk comes from what the mod contains: replaced hero models and effects break after a patch
 * far more often than textures or sounds.
 */
export function findSuspects(mods: InstalledMod[] | null, contents: AddonContent[], build: string | null, report: UpdateReport | null): Suspect[] {
  if (!mods || !build) return [];
  const byFile = new Map(contents.map((c) => [c.name, c.content]));
  const updates = new Set((report?.updates ?? []).map((u) => u.mod_id));
  const out: Suspect[] = [];
  for (const m of mods) {
    if (!m.enabled || !m.game_build || m.game_build === build) continue;
    const kinds = new Set<string>();
    let seen = false;
    for (const f of m.files) {
      const c = byFile.get(f.name.replace(/\.disabled$/i, ""));
      if (!c) continue;
      seen = true;
      for (const k of c.kinds) kinds.add(k.kind);
      if ((c.model_heroes?.length ?? 0) > 0) kinds.add("models");
    }
    const list = [...kinds];
    // content unknown: treat as medium rather than hide a possibly broken mod
    const risk: Risk = list.some((k) => FRAGILE.has(k)) ? "high" : !seen || list.some((k) => MEDIUM.has(k)) ? "mid" : "low";
    out.push({ mod: m, risk, kinds: list, update: updates.has(m.id) });
  }
  return out.sort((a, b) => ORDER[a.risk] - ORDER[b.risk] || Number(b.update) - Number(a.update) || a.mod.name.localeCompare(b.mod.name));
}
