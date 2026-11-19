import { useEffect, useMemo, useRef, useState } from "react";
import type { TFunction } from "i18next";
import { call, type Content, type ForeignAddon, type ModDetails } from "./api";
import { mergeContent, prettyFileName } from "./content";
import { loadCachedMods, saveCachedMods, staleIds, toCached, type CachedMod } from "./modcache";

/** One mod installed by another manager. A mod can ship several VPK files, so files are grouped by catalog id. */
export type ForeignGroup = {
  key: string;
  id: string | null;
  files: ForeignAddon[];
  content: Content | null;
  size: number;
  /** How many of its files the game loads. */
  enabled: number;
};

export type GroupState = "on" | "off" | "partial";
export const groupState = (g: ForeignGroup): GroupState => (g.enabled === 0 ? "off" : g.enabled === g.files.length ? "on" : "partial");

export function groupForeign(foreign: ForeignAddon[]): ForeignGroup[] {
  const byKey = new Map<string, ForeignAddon[]>();
  for (const f of foreign) {
    // The key must not change when the mod is switched off and on (the file gets or loses `.disabled`).
    const k = f.remote_id ?? `file:${f.name.replace(/\.disabled$/, "")}`;
    byKey.set(k, [...(byKey.get(k) ?? []), f]);
  }
  return [...byKey.entries()].map(([key, files]) => ({
    key,
    id: files[0].remote_id,
    files,
    content: mergeContent(files.flatMap((f) => (f.content ? [f.content] : []))),
    size: files.reduce((sum, f) => sum + f.size, 0),
    enabled: files.filter((f) => f.enabled).length,
  }));
}

/** What to call a group: the catalog name, else the hero, else an honest "unidentified". */
export function groupTitle(g: ForeignGroup, known: Record<string, CachedMod | null>, t: TFunction): string {
  const mod = g.id ? known[g.id] : undefined;
  if (mod?.name) return mod.name;
  if (g.id) return t("library.modId", { id: g.id });
  return g.content?.heroes[0]?.name ?? `${t("library.unknown")} · ${prettyFileName(g.files[0].name)}`;
}

/**
 * Groups of foreign addons plus their catalog info. Names and icons come from a cache kept between
 * runs, so they appear at once; only ids never seen before are looked up (8 at a time).
 */
export function useForeignMods(foreign: ForeignAddon[], extraIds: string[] = []) {
  const [known, setKnown] = useState<Record<string, CachedMod | null>>(() => loadCachedMods());
  const extraKey = extraIds.join(",");
  const ids = useMemo(() => [...new Set([...foreign.map((f) => f.remote_id), ...extraKey.split(",")].filter((id): id is string => !!id && /^\d+$/.test(id)))], [foreign, extraKey]);

  // Ids already asked about, so a refresh while lookups are running does not ask again.
  const asked = useRef(new Set<string>());
  useEffect(() => {
    const stale = staleIds();
    const todo = ids.filter((id) => (!(id in known) || stale.has(id)) && !asked.current.has(id));
    todo.forEach((id) => asked.current.add(id));
    (async () => {
      const POOL = 8;
      for (let i = 0; i < todo.length; i += POOL) {
        const batch = await Promise.all(todo.slice(i, i + POOL).map((id) => call<ModDetails>("mod_details", { id }).then((d) => [id, toCached(d.summary)] as const, () => [id, null] as const)));
        saveCachedMods(Object.fromEntries(batch.filter((b): b is readonly [string, CachedMod] => b[1] !== null)));
        // a failed refresh keeps what was known before
        setKnown((prev) => ({ ...prev, ...Object.fromEntries(batch.filter(([id, m]) => m !== null || !(id in prev))) }));
      }
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps -- only ids not asked about yet are looked up
  }, [ids]);

  // Alphabetical, whatever the state: a mod must not jump to another place when it is switched on or off.
  const groups = useMemo(() => groupForeign(foreign).sort((a, b) => (known[a.id ?? ""]?.name ?? a.key).localeCompare(known[b.id ?? ""]?.name ?? b.key)), [foreign, known]);
  return { groups, known };
}
