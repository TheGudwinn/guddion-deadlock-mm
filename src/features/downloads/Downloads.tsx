import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { call, type HistoryEntry, type QueueRow } from "../../lib/api";
import { formatBytes, humanizeError } from "../../lib/format";
import EmptyState from "../../components/EmptyState";

type Props = {
  rows: QueueRow[];
  onPause: (id: number) => void;
  onResume: (id: number) => void;
  onCancel: (id: number) => void;
  onRemove: (id: number) => void;
  onPauseAll: () => void;
  onResumeAll: () => void;
  onClear: () => void;
  onGoCatalog: () => void;
  /** Changes when something was updated, so the history is read again. */
  historyKey: number;
};

const ACTIVE = ["queued", "downloading", "paused", "installing"];

export default function Downloads({ rows, onPause, onResume, onCancel, onRemove, onPauseAll, onResumeAll, onClear, onGoCatalog, historyKey }: Props) {
  const { t } = useTranslation();
  const [history, setHistory] = useState<HistoryEntry[]>([]);
  // what the queue shows changes the history too (an install ends); read it again then
  const signature = rows.map((r) => `${r.id}:${r.state}`).join(",");
  useEffect(() => {
    let alive = true;
    call<HistoryEntry[]>("list_history").then(
      (h) => alive && setHistory(h),
      () => {},
    );
    return () => {
      alive = false;
    };
  }, [signature, historyKey]);

  const clearHistory = async () => {
    await call("clear_history").catch(() => {});
    setHistory([]);
  };

  const canPause = rows.some((r) => r.state === "queued" || r.state === "downloading");
  const canResume = rows.some((r) => r.state === "paused");
  const canClear = rows.some((r) => r.state === "done" || r.state === "cancelled");

  return (
    <section className="view">
      <header className="view-head">
        <div className="view-title-row">
          <h1 className="view-title">{t("downloads.title")}</h1>
          <div className="head-actions">
            <button className="btn" disabled={!canPause} onClick={onPauseAll}>
              {t("downloads.pauseAll")}
            </button>
            <button className="btn" disabled={!canResume} onClick={onResumeAll}>
              {t("downloads.resumeAll")}
            </button>
            <button className="btn btn-quiet" disabled={!canClear} onClick={onClear}>
              {t("downloads.clear")}
            </button>
          </div>
        </div>
        <p className="view-sub">{t("downloads.subtitle")}</p>
      </header>

      {rows.length === 0 && history.length === 0 && (
        <EmptyState title={t("downloads.emptyTitle")} text={t("downloads.empty")}>
          <button className="btn btn-primary" onClick={onGoCatalog}>
            {t("library.toCatalog")}
          </button>
        </EmptyState>
      )}

      <ul className="dls">
        {[...rows].reverse().map((r) => {
          const pct = r.total ? Math.min(100, Math.round((r.downloaded / r.total) * 100)) : null;
          const showBar = r.state === "downloading" || r.state === "paused" || r.state === "queued";
          return (
            <li key={r.id} className={`dl dl-${r.state}`}>
              <div className="dl-main">
                <div className="dl-name">{r.name}</div>
                <div className="slot-meta">
                  <span className={`dl-state dl-state-${r.state}`}>{t(`downloads.state.${r.state}`)}</span>
                  {" · "}
                  {r.file_name}
                  {r.state === "done" && r.slot != null && ` · ${t("library.slot").toLowerCase()} ${String(r.slot).padStart(2, "0")}`}
                </div>
                {showBar && (
                  <>
                    <div className="bar" role="progressbar" aria-valuenow={pct ?? undefined} aria-valuemin={0} aria-valuemax={100} aria-label={r.name}>
                      <i style={{ width: `${r.state === "queued" ? 0 : (pct ?? 0)}%` }} />
                    </div>
                    <div className="dl-bytes">
                      {r.total ? `${formatBytes(r.downloaded)} / ${formatBytes(r.total)}` : formatBytes(r.downloaded)}
                      {pct != null && r.state !== "queued" && ` · ${pct}%`}
                    </div>
                  </>
                )}
                {r.state === "failed" && r.error && <div className="form-error">{humanizeError(t, r.error)}</div>}
                {r.state === "done" && (r.overlaps ?? 0) > 0 && <div className="note note-warn">{t("activity.overlaps", { count: r.overlaps })}</div>}
              </div>
              <div className="dl-actions">
                {(r.state === "queued" || r.state === "downloading") && (
                  <button className="btn" onClick={() => onPause(r.id)}>
                    {t("downloads.pause")}
                  </button>
                )}
                {r.state === "paused" && (
                  <button className="btn" onClick={() => onResume(r.id)}>
                    {t("downloads.resume")}
                  </button>
                )}
                {r.state === "failed" && (
                  <button className="btn" onClick={() => onResume(r.id)}>
                    {t("downloads.retry")}
                  </button>
                )}
                {ACTIVE.includes(r.state) && r.state !== "installing" && (
                  <button className="btn btn-quiet" onClick={() => onCancel(r.id)}>
                    {t("downloads.cancel")}
                  </button>
                )}
                {!ACTIVE.includes(r.state) && (
                  <button className="btn btn-quiet" onClick={() => onRemove(r.id)}>
                    {t("downloads.remove")}
                  </button>
                )}
              </div>
            </li>
          );
        })}
      </ul>

      {history.length > 0 && (
        <>
          <div className="view-title-row hist-head">
            <h2 className="block-title">{t("downloads.history")}</h2>
            <button className="btn btn-quiet btn-sm" onClick={() => void clearHistory()}>
              {t("downloads.historyClear")}
            </button>
          </div>
          <p className="note">{t("downloads.historyNote")}</p>
          <ul className="hist">
            {history.map((h, i) => (
              <li key={`${h.at}-${i}`} className={`hist-row hist-${h.state}`}>
                <span className="hist-mark" aria-hidden="true">
                  {h.state === "done" ? "✓" : "✕"}
                </span>
                <span className="hist-text">
                  <span className="hist-name">{h.name}</span>
                  <span className="hist-sub">
                    {h.state === "failed" ? t("downloads.historyFailed") : h.kind === "update" ? t("downloads.historyUpdated") : t("downloads.historyInstalled")}
                    {h.file && ` · ${h.file}`}
                    {h.state === "failed" && h.detail && ` · ${humanizeError(t, h.detail)}`}
                  </span>
                </span>
                <time className="hist-time" dateTime={new Date(h.at).toISOString()}>
                  {new Date(h.at).toLocaleString()}
                </time>
              </li>
            ))}
          </ul>
        </>
      )}
    </section>
  );
}
