import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { ForeignAddon, InstalledMod, LastAction, ModUpdate, UpdateReport } from "../../lib/api";
import { formatBytes } from "../../lib/format";

type Props = {
  mods: InstalledMod[] | null;
  foreign: ForeignAddon[];
  last: LastAction | null;
  busy: boolean;
  onToggle: (id: number, enabled: boolean) => void;
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

export default function Library({ mods, foreign, last, busy, onToggle, onRemove, onUndo, onReorder, gameBuild, report, checking, onCheckUpdates, onUpdate, onGoCatalog }: Props) {
  const { t } = useTranslation();
  const [confirm, setConfirm] = useState<number | null>(null);
  const [drag, setDrag] = useState<number | null>(null); // index being dragged
  const [over, setOver] = useState<number | null>(null); // index it would land on

  const updateFor = (id: number): ModUpdate | undefined => report?.updates.find((u) => u.mod_id === id);
  const stale = gameBuild ? (mods ?? []).filter((m) => m.game_build && m.game_build !== gameBuild) : [];

  const move = (from: number, to: number) => {
    if (!mods || from === to || to < 0 || to >= mods.length) return;
    const ids = mods.map((m) => m.id);
    const [id] = ids.splice(from, 1);
    ids.splice(to, 0, id);
    onReorder(ids);
  };

  const undoLabel = last
    ? t("library.undo", {
        what: t(`action.${["install", "uninstall", "toggle", "profile", "reorder", "update"].includes(last.kind) ? last.kind : "generic"}`, { name: last.mod_name ?? "" }),
      })
    : null;

  return (
    <section className="view">
      <header className="view-head">
        <div className="view-title-row">
          <h1 className="view-title">{t("library.title")}</h1>
          <div className="head-actions">
            {mods && mods.length > 0 && (
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
        {mods && mods.length > 1 && <p className="note">{t("library.orderNote")}</p>}
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

      {mods && mods.length === 0 && (
        <div className="empty">
          <p>{t("library.empty")}</p>
          <button className="btn btn-primary" onClick={onGoCatalog}>
            {t("library.toCatalog")}
          </button>
        </div>
      )}

      {mods && mods.length > 0 && (
        <ol className="rack">
          {mods.map((m, i) => (
            <li
              key={m.id}
              className={`slot-row ${m.enabled ? "" : "slot-off"} ${drag === i ? "dragging" : ""} ${over === i && drag !== null && drag !== i ? (drag < i ? "drop-after" : "drop-before") : ""}`}
              draggable={!busy && mods.length > 1}
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
              <div className={`plate ${mods.length > 1 ? "plate-grab" : ""}`} aria-label={`${t("library.slot")} ${m.slot ?? "?"}`}>
                <small>{t("library.slot")}</small>
                <b>{m.slot != null ? String(m.slot).padStart(2, "0") : "??"}</b>
              </div>
              <div>
                <div className="slot-name">{m.name}</div>
                <div className="slot-meta">
                  {updateFor(m.id) && <span className="pill-update">{t("library.updateAvailable", { version: updateFor(m.id)?.latest_version ? ` ${updateFor(m.id)?.latest_version}` : "" })}</span>}
                  {[m.version && `v${m.version}`, formatBytes(m.files.reduce((s, f) => s + f.size, 0)), t("library.files", { count: m.files.length })]
                    .filter(Boolean)
                    .join(" · ")}
                </div>
              </div>
              <div className="slot-actions">
                {mods.length > 1 && (
                  <span className="mover">
                    <button className="btn-icon" disabled={busy || i === 0} aria-label={`${t("library.moveUp")}: ${m.name}`} title={t("library.moveUp")} onClick={() => move(i, i - 1)}>
                      ▲
                    </button>
                    <button className="btn-icon" disabled={busy || i === mods.length - 1} aria-label={`${t("library.moveDown")}: ${m.name}`} title={t("library.moveDown")} onClick={() => move(i, i + 1)}>
                      ▼
                    </button>
                  </span>
                )}
                {updateFor(m.id) && (
                  <button className="btn btn-primary" disabled={busy} onClick={() => onUpdate(m.id)}>
                    {t("library.update")}
                  </button>
                )}
                <button
                  className="switch"
                  role="switch"
                  aria-checked={m.enabled}
                  aria-label={`${t("library.toggle")}: ${m.name}`}
                  title={m.enabled ? t("library.enabled") : t("library.disabled")}
                  disabled={busy}
                  onClick={() => onToggle(m.id, !m.enabled)}
                />
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
            </li>
          ))}
        </ol>
      )}

      {foreign.length > 0 && (
        <details className="foreign">
          <summary>
            {t("library.foreign")} · {foreign.length}
          </summary>
          <p className="note">{t("library.foreignNote")}</p>
          <ul>
            {foreign.map((f) => (
              <li key={f.name}>
                <span>{f.name}</span>
                <span>{f.entries != null ? `${t("library.entries", { count: f.entries })} · ` : `${t("library.unreadable")} · `}{formatBytes(f.size)}</span>
              </li>
            ))}
          </ul>
        </details>
      )}
    </section>
  );
}
