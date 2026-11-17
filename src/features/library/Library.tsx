import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { AddonContent, Content, InstalledMod, LastAction, ModUpdate, UpdateReport } from "../../lib/api";
import { groupState, groupTitle, type ForeignGroup, type GroupState } from "../../lib/foreign";
import { smallThumb, type CachedMod } from "../../lib/modcache";
import { contentForFiles, mainKinds } from "../../lib/content";
import { formatBytes } from "../../lib/format";
import { camStateOf, isFixable, shortName, type CamState, type Compat } from "../../lib/compat";
import StateMark from "../../components/StateMark";
import { Tags, Peek } from "./ContentView";

type Props = {
  mods: InstalledMod[] | null;
  /** Mods installed by other managers, one entry per mod, and what the catalog knows about them. */
  groups: ForeignGroup[];
  known: Record<string, CachedMod | null>;
  /** What each addon installed by this app contains. */
  contents: AddonContent[];
  last: LastAction | null;
  busy: boolean;
  onToggle: (id: number, enabled: boolean) => void;
  onToggleForeign: (g: ForeignGroup, enabled: boolean) => void;
  onRemove: (id: number) => void;
  onUndo: () => void;
  onReorder: (ids: number[]) => void;
  /** Current game build, to flag mods installed on an older one. */
  gameBuild: string | null;
  report: UpdateReport | null;
  checking: boolean;
  onCheckUpdates: () => void;
  onUpdate: (id: number) => void;
  onGoCatalog: () => void;
  /** Camera check of the hero models inside the mods. */
  compat: Compat;
  gameFound: boolean;
};

/** One tile of the library, whoever installed the mod. */
type Item = {
  key: string;
  title: string;
  thumb: string | null;
  sub: string;
  state: GroupState;
  content: Content | null;
  size: number;
  hero: string | null;
  kinds: string[];
  own?: InstalledMod;
  group?: ForeignGroup;
  update?: ModUpdate;
  /** Camera check: needs the repair, or has been repaired. */
  cam?: CamState;
  error?: string;
  /** The catalog (GameBanana) knows this mod. */
  found: boolean;
  /** Position in the load order for our own mods. */
  order: number;
};

type Status = "all" | "on" | "off";
type Sort = "load" | "name" | "size" | "state";

/** Catalog thumbnail at the size the catalog gives (a tile is wider than the 100 px preview). Defined here, not inside the page, so tiles keep their image when the list re-renders. */
function Thumb({ url }: { url: string }) {
  return (
    <img
      className="tile-img"
      src={url}
      alt=""
      loading="lazy"
      decoding="async"
      onError={(e) => {
        // the full-size picture is the default; the small one is only a fallback
        const img = e.currentTarget;
        const small = smallThumb(url);
        if (img.src !== small) img.src = small;
      }}
    />
  );
}

export default function Library({ mods, groups, known, contents, last, busy, onToggle, onToggleForeign, onRemove, onUndo, onReorder, gameBuild, report, checking, onCheckUpdates, onUpdate, onGoCatalog, compat, gameFound }: Props) {
  const { t } = useTranslation();
  const [confirm, setConfirm] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [status, setStatus] = useState<Status>("on");
  const [kind, setKind] = useState("");
  const [hero, setHero] = useState("");
  const [onlyUpdates, setOnlyUpdates] = useState(false);
  const [sort, setSort] = useState<Sort>("name");
  const [open, setOpen] = useState<Set<string>>(new Set()); // tiles whose "what is inside" panel is expanded

  const toggleOpen = (key: string) =>
    setOpen((prev) => {
      const next = new Set(prev);
      if (!next.delete(key)) next.add(key);
      return next;
    });

  const own = mods ?? [];
  // the check is cheap to repeat and reads the mod files themselves, so it follows what is installed
  const filesKey = useMemo(() => [...own.flatMap((m) => m.files.map((f) => `${f.name}:${f.enabled}`)), ...groups.flatMap((g) => g.files.map((f) => `${f.name}:${f.enabled}`))].sort().join("|"), [own, groups]);
  const { scan: scanCompat } = compat;
  useEffect(() => {
    if (gameFound && mods && filesKey) void scanCompat();
  }, [gameFound, mods, filesKey, scanCompat]);
  const updateFor = (id: number): ModUpdate | undefined => report?.updates.find((u) => u.mod_id === id);

  const items: Item[] = useMemo(() => {
    const list: Item[] = [];
    own.forEach((m, i) => {
      const c = contentForFiles(contents, m.files.map((f) => f.name));
      const cat = m.remote_id ? known[m.remote_id] : null;
      const size = m.files.reduce((s, f) => s + f.size, 0);
      list.push({
        key: `mod-${m.id}`,
        title: m.name,
        thumb: cat?.thumbnail ?? null,
        sub: [m.version && `v${m.version}`, formatBytes(size), m.files.length > 1 && t("library.files", { count: m.files.length })].filter(Boolean).join(" · "),
        state: m.enabled ? "on" : "off",
        content: c,
        size,
        hero: c?.heroes[0]?.name ?? null,
        kinds: c ? mainKinds(c, 6) : [],
        found: !!cat,
        own: m,
        update: updateFor(m.id),
        cam: camStateOf(compat.report, m.files.map((f) => f.name)) ?? undefined,
        order: i,
      });
    });
    groups.forEach((g, i) => {
      const mod = g.id ? known[g.id] : null;
      const hint = !g.id && g.content ? [...new Set(g.content.sample.map((p) => p.split("/").pop() ?? p))].slice(0, 2).join(", ") : null;
      const err = g.files.find((f) => f.error)?.error;
      list.push({
        key: `foreign-${g.key}`,
        title: groupTitle(g, known, t),
        thumb: mod?.thumbnail ?? null,
        sub: [mod ? [mod.author, mod.category].filter(Boolean).join(" · ") : hint, g.files.length > 1 ? t("library.files", { count: g.files.length }) : null, formatBytes(g.size)].filter(Boolean).join(" · "),
        state: groupState(g),
        content: g.content,
        size: g.size,
        hero: g.content?.heroes[0]?.name ?? null,
        kinds: g.content ? mainKinds(g.content, 6) : [],
        found: !!mod,
        group: g,
        error: err ?? undefined,
        cam: camStateOf(compat.report, g.files.map((f) => f.name)) ?? undefined,
        order: own.length + i,
      });
    });
    return list;
    // eslint-disable-next-line react-hooks/exhaustive-deps -- updateFor only reads `report`
  }, [own, groups, known, contents, report, compat.report, t]);

  const heroes = useMemo(() => [...new Set(items.map((i) => i.hero).filter((h): h is string => !!h))].sort((a, b) => a.localeCompare(b)), [items]);
  const kinds = useMemo(() => [...new Set(items.flatMap((i) => i.kinds))].sort(), [items]);

  const q = query.trim().toLowerCase();
  const visible = useMemo(() => {
    const hay = (i: Item) =>
      [i.title, i.sub, ...(i.content?.heroes.map((h) => `${h.name} ${h.code}`) ?? []), ...i.kinds.map((k) => t(`kind.${k}`)), ...(i.own?.files.map((f) => f.name) ?? []), ...(i.group?.files.map((f) => f.name) ?? []), i.group?.id ?? ""]
        .join(" ")
        .toLowerCase();
    const out = items.filter(
      (i) =>
        (!q || hay(i).includes(q)) &&
        (status === "all" || (status === "on" ? i.state !== "off" : i.state === "off")) &&
        (!kind || i.kinds.includes(kind)) &&
        (!hero || i.hero === hero) &&
        (!onlyUpdates || !!i.update),
    );
    // mods the catalog does not know go after the ones it knows
    const known1 = (a: Item, b: Item) => Number(b.found) - Number(a.found);
    const byName = (a: Item, b: Item) => known1(a, b) || a.title.localeCompare(b.title);
    if (sort === "name") out.sort(byName);
    else if (sort === "size") out.sort((a, b) => known1(a, b) || b.size - a.size || byName(a, b));
    else if (sort === "state") out.sort((a, b) => Number(b.state !== "off") - Number(a.state !== "off") || byName(a, b));
    else out.sort((a, b) => a.order - b.order);
    return out;
  }, [items, q, status, kind, hero, onlyUpdates, sort, t]);

  const filtered = !!q || status !== "on" || !!kind || !!hero || onlyUpdates;
  const reorderable = !filtered && sort === "load";
  const total = items.length;
  const totals = useMemo(
    () => ({ count: items.length, enabled: items.filter((i) => i.state !== "off").length, bytes: items.reduce((s, i) => s + i.size, 0) }),
    [items],
  );
  // mods that still need the camera repair, and the package that is written but not switched on
  const camMods = new Set((compat.report?.findings ?? []).filter((f) => f.kind.startsWith("camera_")).flatMap((f) => f.mods.map(shortName)));
  const canRepair = compat.report?.findings.some(isFixable) ?? false;
  const stale = gameBuild ? own.filter((m) => m.game_build && m.game_build !== gameBuild) : [];

  const move = (from: number, to: number) => {
    if (!mods || from === to || to < 0 || to >= mods.length) return;
    const ids = mods.map((m) => m.id);
    const [id] = ids.splice(from, 1);
    ids.splice(to, 0, id);
    onReorder(ids);
  };

  const reset = () => {
    setQuery("");
    setStatus("on");
    setKind("");
    setHero("");
    setOnlyUpdates(false);
  };

  const undoLabel = last
    ? t("library.undo", {
        what: t(`action.${["install", "uninstall", "toggle", "profile", "reorder", "update", "foreign"].includes(last.kind) ? last.kind : "generic"}`, { name: last.mod_name ?? "" }),
      })
    : null;

  const renderTile = (it: Item) => {
    const expanded = open.has(it.key);
    const m = it.own;
    const g = it.group;
    const i = m ? own.findIndex((x) => x.id === m.id) : -1;
    const toggle = () => (m ? onToggle(m.id, it.state !== "on") : g && onToggleForeign(g, it.state !== "on"));
    const label = `${it.state === "on" ? t("home.disable") : t("home.enable")}: ${it.title}`;
    return (
      <li key={it.key} className={`tile ${it.state === "off" ? "tile-off" : ""} ${expanded ? "tile-wide" : ""}`}>
        <div className="tile-cover">
          {it.thumb ? <Thumb url={it.thumb} /> : <div className="tile-img tile-img-empty" aria-hidden>{it.hero?.[0] ?? it.title[0]}</div>}
          <div className="tile-mark">
            <StateMark state={it.state} label={label} disabled={busy || !!it.error} onClick={toggle} />
          </div>
          {m && <span className="tile-slot">{m.slot != null ? String(m.slot).padStart(2, "0") : "??"}</span>}
          {it.update && <span className="tile-badge">{t("library.updateAvailable", { version: it.update.latest_version ? ` ${it.update.latest_version}` : "" })}</span>}
          {it.cam && (
            <span className={`tile-cam tile-cam-${it.cam.state} ${it.update ? "tile-cam-up" : ""}`} title={[t(`library.cam_${it.cam.state}_text`), ...it.cam.models].join(String.fromCharCode(10))}>
              {t(`library.cam_${it.cam.state}`)}
            </span>
          )}
        </div>
        <div className="tile-body">
          <div className="tile-title" title={it.title}>
            {it.title}
          </div>
          <div className="slot-meta">{it.error ? <span className="slot-error">{t("library.unreadable")}: {it.error}</span> : it.sub}</div>
          {it.content ? <Tags content={it.content} skipHero={!it.thumb && !!it.hero && !!g && !g.id} /> : <div className="slot-meta">{t("library.noContent")}</div>}
        </div>
        <div className="tile-actions">
          {it.content && (
            <button className="btn-icon btn-chevron" aria-expanded={expanded} aria-label={`${t("library.showInside")}: ${it.title}`} title={t("library.showInside")} onClick={() => toggleOpen(it.key)}>
              ▾
            </button>
          )}
          {m && reorderable && own.length > 1 && (
            <span className="mover">
              <button className="btn-icon" disabled={busy || i === 0} aria-label={`${t("library.moveUp")}: ${m.name}`} title={t("library.moveUp")} onClick={() => move(i, i - 1)}>
                ◀
              </button>
              <button className="btn-icon" disabled={busy || i === own.length - 1} aria-label={`${t("library.moveDown")}: ${m.name}`} title={t("library.moveDown")} onClick={() => move(i, i + 1)}>
                ▶
              </button>
            </span>
          )}
          {m && it.update && (
            <button className="btn btn-primary" disabled={busy} onClick={() => onUpdate(m.id)}>
              {t("library.update")}
            </button>
          )}
          {m &&
            (confirm === it.key ? (
              <>
                <button
                  className="btn btn-danger"
                  disabled={busy}
                  onClick={() => {
                    setConfirm(null);
                    onRemove(m.id);
                  }}
                >
                  {t("library.removeYes")}
                </button>
                <button className="btn btn-quiet" onClick={() => setConfirm(null)}>
                  {t("library.cancel")}
                </button>
              </>
            ) : (
              <button className="btn btn-quiet" disabled={busy} onClick={() => setConfirm(it.key)}>
                {t("library.remove")}
              </button>
            ))}
        </div>
        {expanded && g && g.files.length > 1 && (
          <ul className="group-files">
            {g.files.map((f) => (
              <li key={f.name}>
                <span className={`dot ${f.enabled ? "dot-on" : "dot-off"}`} aria-hidden />
                <span>{f.name}</span>
                <span>{formatBytes(f.size)}</span>
              </li>
            ))}
          </ul>
        )}
        {it.content && expanded && <Peek content={it.content} />}
      </li>
    );
  };

  return (
    <section className="view">
      <header className="view-head">
        <div className="view-title-row">
          <h1 className="view-title">{t("library.title")}</h1>
          <div className="head-actions">
            {total > 0 && gameFound && (
              <button className="btn" disabled={busy || compat.scanning || compat.repairing} onClick={() => void compat.scan()}>
                {compat.scanning ? t("fixes.compat.scanning") : t("library.checkCameras")}
              </button>
            )}
            {own.length > 0 && (
              <button className="btn" disabled={busy || checking} onClick={onCheckUpdates}>
                {checking ? t("library.checking") : t("library.checkUpdates")}
              </button>
            )}
            {undoLabel && (
              <button className="btn" disabled={busy} onClick={onUndo}>
                {undoLabel}
              </button>
            )}
          </div>
        </div>
        <p className="view-sub">{t("library.subtitle")}</p>
        {groups.length > 0 && <p className="note">{t("library.foreignWarn")}</p>}
        {own.length > 1 && <p className="note">{t("library.orderNote")}</p>}
        {report && !checking && (
          <p className="note">
            {report.updates.length > 0 ? t("library.updatesFound", { count: report.updates.length }) : t("library.upToDate", { count: report.checked })}
            {report.failed > 0 && ` ${t("library.checkFailed", { count: report.failed })}`}
            {report.offline && ` ${t("library.checkOffline")}`}
          </p>
        )}
        {compat.error && <p className="form-error">{compat.error}</p>}
        {compat.done && (
          <div className="notice ok-notice" role="status">
            <b>{t("fixes.compat.repaired", { count: compat.done.fixed.length })}</b>
            {compat.done.skipped.map(([m, why]) => (
              <span key={m}>
                {m}: {why}
              </span>
            ))}
            <span>{t("library.camRestart")}</span>
          </div>
        )}
        {canRepair && (
          <div className="notice" role="note">
            <b>{camMods.size > 0 ? t("library.camNeedsTitle", { count: camMods.size }) : t("fixes.compat.package_inactive")}</b>
            <span>{camMods.size > 0 ? t("library.camNeedsText") : t("fixes.compat.package_inactive_text")}</span>
            <div className="row">
              <button className="btn btn-primary" disabled={busy || compat.scanning || compat.repairing} onClick={() => void compat.repair()}>
                {compat.repairing ? "…" : t("fixes.compat.repair")}
              </button>
            </div>
          </div>
        )}
        {stale.length > 0 && (
          <div className="notice" role="note">
            <b>{t("library.patchTitle")}</b>
            <span>{t("library.patchText", { count: stale.length, build: gameBuild })}</span>
          </div>
        )}
      </header>

      {mods && total === 0 && (
        <div className="empty">
          <p>{t("library.empty")}</p>
          <button className="btn btn-primary" onClick={onGoCatalog}>
            {t("library.toCatalog")}
          </button>
        </div>
      )}

      {total > 0 && (
        <>
          <div className="lib-tools">
            <input className="field search" type="search" value={query} placeholder={t("library.search")} aria-label={t("library.search")} onChange={(e) => setQuery(e.target.value)} />
            <span className="lib-totals">{t("library.totals", { count: totals.count, enabled: totals.enabled, size: formatBytes(totals.bytes) })}</span>
          </div>
          <div className="lib-filters" role="group" aria-label={t("library.filters")}>
            <div className="seg" role="radiogroup" aria-label={t("library.status")}>
              {(["all", "on", "off"] as const).map((s) => (
                <button key={s} type="button" role="radio" aria-checked={status === s} className={`seg-btn ${status === s ? "seg-on" : ""}`} onClick={() => setStatus(s)}>
                  {t(`library.status_${s}`)}
                </button>
              ))}
            </div>
            <select className="field" value={hero} aria-label={t("library.filterHero")} onChange={(e) => setHero(e.target.value)}>
              <option value="">{t("library.allHeroes")}</option>
              {heroes.map((h) => (
                <option key={h} value={h}>
                  {h}
                </option>
              ))}
            </select>
            <select className="field" value={kind} aria-label={t("library.filterKind")} onChange={(e) => setKind(e.target.value)}>
              <option value="">{t("library.allKinds")}</option>
              {kinds.map((k) => (
                <option key={k} value={k}>
                  {t(`kind.${k}`)}
                </option>
              ))}
            </select>
            {report && report.updates.length > 0 && (
              <label className="check">
                <input type="checkbox" checked={onlyUpdates} onChange={(e) => setOnlyUpdates(e.target.checked)} />
                {t("library.onlyUpdates")}
              </label>
            )}
            <select className="field" value={sort} aria-label={t("library.sort")} onChange={(e) => setSort(e.target.value as Sort)}>
              {(["load", "name", "size", "state"] as const).map((s) => (
                <option key={s} value={s}>
                  {t(`library.sort_${s}`)}
                </option>
              ))}
            </select>
            {filtered && (
              <button className="btn btn-quiet" onClick={reset}>
                {t("library.resetFilters")}
              </button>
            )}
          </div>
        </>
      )}

      {total > 0 && visible.length === 0 && <p className="note">{t("library.noMatch")}</p>}

      {visible.length > 0 && <ul className="tiles">{visible.map(renderTile)}</ul>}
    </section>
  );
}
