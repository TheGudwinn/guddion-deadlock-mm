import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { AddonContent, Content, InstalledMod, LastAction, ModUpdate, UpdateReport } from "../../lib/api";
import { groupState, groupTitle, type ForeignGroup } from "../../lib/foreign";
import { smallThumb, type CachedMod } from "../../lib/modcache";
import { contentForFiles } from "../../lib/content";
import { formatBytes } from "../../lib/format";
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
};

/** Catalog thumbnail, small size first (the big one is only a fallback). Defined here, not inside the page, so rows keep their image when the list re-renders. */
function Thumb({ url }: { url: string }) {
  return (
    <img
      className="row-thumb"
      src={smallThumb(url)}
      alt=""
      loading="lazy"
      decoding="async"
      onError={(e) => {
        const img = e.currentTarget;
        if (img.src !== url) img.src = url;
      }}
    />
  );
}

export default function Library({ mods, groups, known, contents, last, busy, onToggle, onToggleForeign, onRemove, onUndo, onReorder, gameBuild, report, checking, onCheckUpdates, onUpdate, onGoCatalog }: Props) {
  const { t } = useTranslation();
  const [confirm, setConfirm] = useState<number | null>(null);
  const [drag, setDrag] = useState<number | null>(null); // index being dragged
  const [over, setOver] = useState<number | null>(null); // index it would land on
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState<Set<string>>(new Set()); // rows whose "what is inside" panel is expanded

  const toggleOpen = (key: string) =>
    setOpen((prev) => {
      const next = new Set(prev);
      if (!next.delete(key)) next.add(key);
      return next;
    });

  const modContent = useMemo(() => {
    const m = new Map<number, Content | null>();
    for (const mod of mods ?? []) m.set(mod.id, contentForFiles(contents, mod.files.map((f) => f.name)));
    return m;
  }, [mods, contents]);

  /** Everything a row can be found by: name, author, version, file names, hero names, content kinds. */
  const haystack = (name: string, c: Content | null | undefined, extra: string[] = []) =>
    [name, ...extra, ...(c?.heroes.map((h) => `${h.name} ${h.code}`) ?? []), ...(c?.kinds.map((k) => t(`kind.${k.kind}`)) ?? [])].join(" ").toLowerCase();
  const q = query.trim().toLowerCase();
  const own = mods ?? [];
  const visibleMods = own.filter((m) => !q || haystack(m.name, modContent.get(m.id), [m.version ?? "", ...m.files.map((f) => f.name)]).includes(q));
  const visibleGroups = groups.filter((g) => {
    const mod = g.id ? known[g.id] : null;
    return !q || haystack(groupTitle(g, known, t), g.content, [mod?.author ?? "", mod?.category ?? "", g.id ?? "", ...g.files.map((f) => f.name)]).includes(q);
  });
  const reorderable = !q; // moving rows inside a filtered view would be ambiguous
  const total = own.length + groups.length;
  const totals = useMemo(
    () => ({
      count: own.length + groups.length,
      enabled: own.filter((m) => m.enabled).length + groups.filter((g) => g.enabled > 0).length,
      bytes: own.reduce((s, m) => s + m.files.reduce((a, f) => a + f.size, 0), 0) + groups.reduce((s, g) => s + g.size, 0),
    }),
    [own, groups],
  );

  const updateFor = (id: number): ModUpdate | undefined => report?.updates.find((u) => u.mod_id === id);
  const stale = gameBuild ? own.filter((m) => m.game_build && m.game_build !== gameBuild) : [];

  const move = (from: number, to: number) => {
    if (!mods || from === to || to < 0 || to >= mods.length) return;
    const ids = mods.map((m) => m.id);
    const [id] = ids.splice(from, 1);
    ids.splice(to, 0, id);
    onReorder(ids);
  };

  const undoLabel = last
    ? t("library.undo", {
        what: t(`action.${["install", "uninstall", "toggle", "profile", "reorder", "update", "foreign"].includes(last.kind) ? last.kind : "generic"}`, { name: last.mod_name ?? "" }),
      })
    : null;

  const renderOwn = (m: InstalledMod) => {
    const i = own.findIndex((x) => x.id === m.id); // position in the full list; filtering must not change it
    const c = modContent.get(m.id) ?? null;
    const key = `mod-${m.id}`;
    const expanded = open.has(key);
    const upd = updateFor(m.id);
    return (
      <li
        key={key}
        className={`slot-row ${m.enabled ? "" : "slot-off"} ${drag === i ? "dragging" : ""} ${over === i && drag !== null && drag !== i ? (drag < i ? "drop-after" : "drop-before") : ""}`}
        draggable={!busy && reorderable && own.length > 1}
        onDragStart={(e) => {
          setDrag(i);
          e.dataTransfer.effectAllowed = "move";
          e.dataTransfer.setData("text/plain", String(m.id));
        }}
        onDragOver={(e) => {
          if (drag === null) return;
          e.preventDefault();
          e.dataTransfer.dropEffect = "move";
          if (over !== i) setOver(i);
        }}
        onDrop={(e) => {
          e.preventDefault();
          if (drag !== null) move(drag, i);
          setDrag(null);
          setOver(null);
        }}
        onDragEnd={() => {
          setDrag(null);
          setOver(null);
        }}
      >
        <div className={`plate ${own.length > 1 && reorderable ? "plate-grab" : ""}`} aria-label={`${t("library.slot")} ${m.slot ?? "?"}`}>
          <small>{t("library.slot")}</small>
          <b>{m.slot != null ? String(m.slot).padStart(2, "0") : "??"}</b>
        </div>
        <div className="slot-main">
          <div className="slot-name">{m.name}</div>
          <div className="slot-meta">
            {upd && <span className="pill-update">{t("library.updateAvailable", { version: upd.latest_version ? ` ${upd.latest_version}` : "" })}</span>}
            {[m.version && `v${m.version}`, formatBytes(m.files.reduce((s, f) => s + f.size, 0)), m.files.length > 1 && t("library.files", { count: m.files.length })].filter(Boolean).join(" · ")}
          </div>
          {c ? <Tags content={c} /> : <div className="slot-meta">{t("library.noContent")}</div>}
        </div>
        <div className="slot-actions">
          {c && (
            <button className="btn-icon btn-chevron" aria-expanded={expanded} aria-label={`${t("library.showInside")}: ${m.name}`} title={t("library.showInside")} onClick={() => toggleOpen(key)}>
              ▾
            </button>
          )}
          {own.length > 1 && reorderable && (
            <span className="mover">
              <button className="btn-icon" disabled={busy || i === 0} aria-label={`${t("library.moveUp")}: ${m.name}`} title={t("library.moveUp")} onClick={() => move(i, i - 1)}>
                ▲
              </button>
              <button className="btn-icon" disabled={busy || i === own.length - 1} aria-label={`${t("library.moveDown")}: ${m.name}`} title={t("library.moveDown")} onClick={() => move(i, i + 1)}>
                ▼
              </button>
            </span>
          )}
          {upd && (
            <button className="btn btn-primary" disabled={busy} onClick={() => onUpdate(m.id)}>
              {t("library.update")}
            </button>
          )}
          <StateMark state={m.enabled ? "on" : "off"} label={`${m.enabled ? t("home.disable") : t("home.enable")}: ${m.name}`} disabled={busy} onClick={() => onToggle(m.id, !m.enabled)} />
          {confirm === m.id ? (
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
            <button className="btn btn-quiet" disabled={busy} onClick={() => setConfirm(m.id)}>
              {t("library.remove")}
            </button>
          )}
        </div>
        {c && expanded && <Peek content={c} />}
      </li>
    );
  };

  const renderGroup = (g: ForeignGroup) => {
    const key = `foreign-${g.key}`;
    const expanded = open.has(key);
    const mod = g.id ? known[g.id] : undefined;
    const hero = g.content?.heroes[0]?.name;
    const title = groupTitle(g, known, t);
    const state = groupState(g);
    const errors = g.files.filter((f) => f.error);
    // Without a name, show a couple of file names from inside so the user can recognise it.
    const hint = !g.id && g.content ? [...new Set(g.content.sample.map((p) => p.split("/").pop() ?? p))].slice(0, 2).join(", ") : null;
    const sub = [mod ? [mod.author, mod.category].filter(Boolean).join(" · ") : hint, g.files.length > 1 ? t("library.files", { count: g.files.length }) : null, formatBytes(g.size)].filter(Boolean).join(" · ");
    return (
      <li key={key} className={`slot-row ${state === "off" ? "slot-off" : ""}`}>
        {mod?.thumbnail ? <Thumb url={mod.thumbnail} /> : <div className="row-thumb row-thumb-empty" aria-hidden />}
        <div className="slot-main">
          <div className="slot-name">{title}</div>
          <div className="slot-meta">
            {errors.length > 0 ? (
              <span className="slot-error">
                {t("library.unreadable")}: {errors[0].error}
              </span>
            ) : (
              sub
            )}
          </div>
          {g.content && <Tags content={g.content} skipHero={!mod && !g.id && !!hero} />}
        </div>
        <div className="slot-actions">
          {g.content && (
            <button className="btn-icon btn-chevron" aria-expanded={expanded} aria-label={`${t("library.showInside")}: ${title}`} title={t("library.showInside")} onClick={() => toggleOpen(key)}>
              ▾
            </button>
          )}
          <StateMark
            state={state}
            label={`${state === "on" ? t("home.disable") : t("home.enable")}: ${title}`}
            disabled={busy || errors.length > 0}
            onClick={() => onToggleForeign(g, state !== "on")}
          />
        </div>
        {expanded && g.files.length > 1 && (
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
        {g.content && expanded && <Peek content={g.content} />}
      </li>
    );
  };

  return (
    <section className="view">
      <header className="view-head">
        <div className="view-title-row">
          <h1 className="view-title">{t("library.title")}</h1>
          <div className="head-actions">
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
        <div className="lib-tools">
          <input className="field search" type="search" value={query} placeholder={t("library.search")} aria-label={t("library.search")} onChange={(e) => setQuery(e.target.value)} />
          <span className="lib-totals">{t("library.totals", { count: totals.count, enabled: totals.enabled, size: formatBytes(totals.bytes) })}</span>
        </div>
      )}

      {total > 0 && visibleMods.length + visibleGroups.length === 0 && <p className="note">{t("library.noMatch")}</p>}

      {visibleMods.length + visibleGroups.length > 0 && (
        <ol className="rack">
          {visibleMods.map(renderOwn)}
          {visibleGroups.map(renderGroup)}
        </ol>
      )}
    </section>
  );
}
