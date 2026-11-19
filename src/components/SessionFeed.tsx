import { useTranslation } from "react-i18next";
import type { QueueRow } from "../lib/api";

/** One line of what happened to mods since the app was started. */
export type FeedEntry = {
  key: string;
  kind: "download" | "update";
  name: string;
  state: "active" | "done" | "failed";
  /** Download queue row, for the live state and progress of a running download. */
  rowId?: number;
  at: number;
};

type Props = {
  entries: FeedEntry[];
  queue: QueueRow[];
  onOpen: (view: "downloads" | "library") => void;
};

const SHOWN = 6;

/** What mods were downloaded, updated or are being downloaded since the app was started, in the left panel. */
export default function SessionFeed({ entries, queue, onOpen }: Props) {
  const { t } = useTranslation();
  if (entries.length === 0) return null;

  // running ones first, then the newest
  const list = [...entries].sort((a, b) => Number(b.state === "active") - Number(a.state === "active") || b.at - a.at).slice(0, SHOWN);

  const status = (e: FeedEntry): { text: string; pct: number | null } => {
    if (e.state === "failed") return { text: t("feed.failed"), pct: null };
    if (e.state === "done") return { text: e.kind === "update" ? t("feed.updated") : t("feed.installed"), pct: null };
    if (e.kind === "update") return { text: t("feed.updating"), pct: null };
    const row = queue.find((r) => r.id === e.rowId);
    if (row?.state === "queued") return { text: t("feed.queued"), pct: null };
    if (row?.state === "paused") return { text: t("feed.paused"), pct: null };
    if (row?.state === "installing") return { text: t("feed.installing"), pct: null };
    const pct = row?.total ? Math.min(100, Math.round((row.downloaded / row.total) * 100)) : null;
    return { text: pct != null ? t("feed.downloadingPct", { pct }) : t("feed.downloading"), pct };
  };

  return (
    <section className="feed" aria-label={t("feed.title")}>
      <h2 className="feed-title">{t("feed.title")}</h2>
      <ul className="feed-list">
        {list.map((e) => {
          const s = status(e);
          return (
            <li key={e.key}>
              <button className={`feed-row feed-${e.state}`} title={`${e.name}\n${s.text}`} onClick={() => onOpen(e.kind === "download" && e.state === "active" ? "downloads" : "library")}>
                <span className="feed-mark" aria-hidden="true">
                  {e.state === "done" ? "✓" : e.state === "failed" ? "✕" : <i className="feed-spin" />}
                </span>
                <span className="feed-text">
                  <span className="feed-name">{e.name}</span>
                  <span className="feed-sub">{s.text}</span>
                </span>
                {s.pct != null && (
                  <span className="feed-bar" aria-hidden="true">
                    <i style={{ width: `${s.pct}%` }} />
                  </span>
                )}
              </button>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
