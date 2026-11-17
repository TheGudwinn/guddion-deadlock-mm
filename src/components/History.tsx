import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import Icon from "./Icons";
import { useExit } from "../lib/motion";

export type HistoryItem = { id: number; at: number; kind: "ok" | "error"; text: string; sub?: string };

const clock = (at: number) => new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

/** A bell in the top bar. Toasts disappear; this keeps the last events so nothing is missed. */
export default function History({ items, unread, onOpen, onClear }: { items: HistoryItem[]; unread: number; onOpen: () => void; onClear: () => void }) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  const { mounted, closing } = useExit(open, 160);

  useEffect(() => {
    if (!open) return;
    const away = (e: PointerEvent) => {
      if (!box.current?.contains(e.target as Node)) setOpen(false);
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    document.addEventListener("pointerdown", away);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("pointerdown", away);
      document.removeEventListener("keydown", esc);
    };
  }, [open]);

  return (
    <div className="bell" ref={box}>
      <button
        className="btn-icon"
        aria-label={t("history.title")}
        aria-expanded={open}
        title={t("history.title")}
        onClick={() => {
          setOpen((o) => !o);
          if (!open) onOpen();
        }}
      >
        <Icon name="bell" size={18} />
        {unread > 0 && <span className="bell-dot" aria-label={String(unread)} />}
      </button>
      {mounted && (
        <div className={`bell-pop ${closing ? "is-closing" : ""}`} role="dialog" aria-label={t("history.title")}>
          <div className="bell-head">
            <b>{t("history.title")}</b>
            {items.length > 0 && (
              <button className="btn btn-quiet btn-sm" onClick={onClear}>
                {t("history.clear")}
              </button>
            )}
          </div>
          {items.length === 0 ? (
            <p className="note">{t("history.empty")}</p>
          ) : (
            <ul className="bell-list">
              {[...items].reverse().map((i) => (
                <li key={i.id} className={`bell-item bell-${i.kind}`}>
                  <span className="bell-time">{clock(i.at)}</span>
                  <span className="bell-text">
                    {i.text}
                    {i.sub && <small>{i.sub}</small>}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
  );
}
