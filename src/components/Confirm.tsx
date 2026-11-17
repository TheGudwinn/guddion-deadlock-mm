import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

export type ConfirmState = {
  title: string;
  text?: string;
  /** Extra lines under the text, e.g. what exactly will change. */
  details?: string[];
  confirm: string;
  danger?: boolean;
  onConfirm: () => void;
};

/** A small dialog that asks before something that is hard to take back. Escape and the backdrop cancel. */
export default function Confirm({ state, onCancel }: { state: ConfirmState; onCancel: () => void }) {
  const { t } = useTranslation();
  const ok = useRef<HTMLButtonElement>(null);
  const [closing, setClosing] = useState(false);
  const shut = useRef(() => {});
  shut.current = () => {
    if (closing) return;
    setClosing(true);
    window.setTimeout(onCancel, 170);
  };

  useEffect(() => {
    ok.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        shut.current();
      }
    };
    document.addEventListener("keydown", onKey, true);
    return () => document.removeEventListener("keydown", onKey, true);
  }, []);

  return (
    <>
      <div className={`scrim scrim-top ${closing ? "is-closing" : ""}`} onClick={() => shut.current()} />
      <div className={`modal-wrap modal-wrap-top ${closing ? "is-closing" : ""}`} onClick={() => shut.current()}>
        <section className="confirm" role="alertdialog" aria-modal="true" aria-labelledby="confirm-title" onClick={(e) => e.stopPropagation()}>
          <h2 id="confirm-title">{state.title}</h2>
          {state.text && <p>{state.text}</p>}
          {state.details && state.details.length > 0 && (
            <ul className="confirm-list">
              {state.details.map((d) => (
                <li key={d}>{d}</li>
              ))}
            </ul>
          )}
          <div className="confirm-actions">
            <button className="btn btn-quiet" onClick={() => shut.current()}>
              {t("confirm.cancel")}
            </button>
            <button
              ref={ok}
              className={`btn ${state.danger ? "btn-danger-solid" : "btn-primary"}`}
              onClick={() => {
                state.onConfirm();
                shut.current();
              }}
            >
              {state.confirm}
            </button>
          </div>
        </section>
      </div>
    </>
  );
}
