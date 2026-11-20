import type { AddonContent, Content, HeroHit, KindCount } from "./api";

/** Combines the contents of several addon files (a mod can ship more than one VPK) into one. */
export function mergeContent(parts: Content[]): Content | null {
  if (parts.length === 0) return null;
  if (parts.length === 1) return parts[0];
  const kinds = new Map<string, KindCount>();
  const heroes = new Map<string, HeroHit>();
  const modeled = new Map<string, HeroHit>();
  const folders = new Map<string, number>();
  const sample = new Set<string>();
  for (const p of parts) {
    for (const k of p.kinds) {
      const cur = kinds.get(k.kind);
      kinds.set(k.kind, cur ? { ...cur, count: cur.count + k.count, bytes: cur.bytes + k.bytes } : { ...k });
    }
    for (const h of p.heroes) {
      const cur = heroes.get(h.code);
      heroes.set(h.code, cur ? { ...cur, files: cur.files + h.files } : { ...h });
    }
    for (const h of p.model_heroes ?? []) {
      const cur = modeled.get(h.code);
      modeled.set(h.code, cur ? { ...cur, files: cur.files + h.files } : { ...h });
    }
    for (const [f, n] of p.folders) folders.set(f, (folders.get(f) ?? 0) + n);
    for (const s of p.sample) sample.add(s);
  }
  return {
    total_files: parts.reduce((s, p) => s + p.total_files, 0),
    total_bytes: parts.reduce((s, p) => s + p.total_bytes, 0),
    kinds: [...kinds.values()].sort((a, b) => b.count - a.count),
    heroes: [...heroes.values()].sort((a, b) => b.files - a.files).slice(0, 4),
    model_heroes: [...modeled.values()].sort((a, b) => b.files - a.files).slice(0, 4),
    sample: [...sample].sort().slice(0, 80),
    folders: [...folders.entries()].sort((a, b) => b[1] - a[1]).slice(0, 8),
  };
}

/** Contents of the given addon files, looked up by file name (without the `.disabled` suffix). */
export function contentForFiles(all: AddonContent[], names: string[]): Content | null {
  const byName = new Map(all.map((a) => [a.name, a.content]));
  return mergeContent(names.map((n) => byName.get(n)).filter((c): c is Content => !!c));
}

/** `569650_pak06_dir.vpk` -> `569650 pak06`: the file name with the boilerplate removed. */
export function prettyFileName(name: string): string {
  return name.replace(/\.disabled$/i, "").replace(/_dir\.vpk$/i, "").replace(/\.vpk$/i, "").replace(/_/g, " ");
}

/** Short guess of what the content is, from its biggest groups: kind keys, most files first. */
export function mainKinds(c: Content, max = 3): string[] {
  return c.kinds.filter((k) => k.kind !== "other").slice(0, max).map((k) => k.kind);
}
