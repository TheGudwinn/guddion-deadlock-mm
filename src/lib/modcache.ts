import type { Mod } from "./api";

/** Catalog info about mods installed by other managers, kept between runs so icons and names appear at once. */
export type CachedMod = Pick<Mod, "id" | "name" | "author" | "category" | "thumbnail" | "page_url">;

const KEY = "guddion.foreign-mods.v1";
const MAX_AGE_MS = 30 * 24 * 3600 * 1000;

type Stored = Record<string, { at: number; mod: CachedMod }>;

function readAll(): Stored {
  try {
    const raw = localStorage.getItem(KEY);
    return raw ? (JSON.parse(raw) as Stored) : {};
  } catch {
    return {}; // storage blocked or corrupt: behave as an empty cache
  }
}

/** Everything still fresh in the cache, by catalog id. */
export function loadCachedMods(): Record<string, CachedMod> {
  const now = Date.now();
  const out: Record<string, CachedMod> = {};
  for (const [id, v] of Object.entries(readAll())) if (v && now - v.at < MAX_AGE_MS && v.mod) out[id] = v.mod;
  return out;
}

export function saveCachedMods(found: Record<string, CachedMod>) {
  try {
    const all = readAll();
    const now = Date.now();
    for (const [id, mod] of Object.entries(found)) all[id] = { at: now, mod };
    localStorage.setItem(KEY, JSON.stringify(all));
  } catch {
    /* the cache is only an optimisation */
  }
}

export const toCached = (m: Mod): CachedMod => ({ id: m.id, name: m.name, author: m.author, category: m.category, thumbnail: m.thumbnail, page_url: m.page_url });

/** GameBanana also serves a small version of a thumbnail; a 56 px icon does not need the 530 px one. */
export const smallThumb = (url: string) => url.replace("/530-90_", "/100-90_");
