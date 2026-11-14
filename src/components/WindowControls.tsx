import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

/** Minimise / maximise / close for the frameless window. Rendered only inside the native app. */
export default function WindowControls({ onMaximizedChange }: { onMaximizedChange: (m: boolean) => void }) {
  const { t } = useTranslation();
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    let off: (() => void) | undefined;
    let alive = true;
    void (async () => {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const win = getCurrentWindow();
      const sync = async () => {
        const m = await win.isMaximized();
        if (alive) {
          setMaximized(m);
          onMaximizedChange(m);
        }
      };
      await sync();
      off = await win.onResized(() => void sync());
    })();
    return () => {
      alive = false;
      off?.();
    };
  }, [onMaximizedChange]);

  const act = async (what: "minimize" | "toggleMaximize" | "close") => {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    await getCurrentWindow()[what]();
  };

  return (
    <div className="winctl">
      <button className="winbtn" aria-label={t("window.minimize")} title={t("window.minimize")} onClick={() => void act("minimize")}>
        <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
          <path d="M2 6h8" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
        </svg>
      </button>
      <button className="winbtn" aria-label={maximized ? t("window.restore") : t("window.maximize")} title={maximized ? t("window.restore") : t("window.maximize")} onClick={() => void act("toggleMaximize")}>
        {maximized ? (
          <svg width="12" height="12" viewBox="0 0 12 12" fill="none" aria-hidden="true">
            <rect x="2.2" y="3.8" width="6" height="6" rx="1.2" stroke="currentColor" strokeWidth="1.3" />
            <path d="M4 3.8V3a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v4a1 1 0 0 1-1 1h-.8" stroke="currentColor" strokeWidth="1.3" />
          </svg>
        ) : (
          <svg width="12" height="12" viewBox="0 0 12 12" fill="none" aria-hidden="true">
            <rect x="2" y="2" width="8" height="8" rx="1.6" stroke="currentColor" strokeWidth="1.3" />
          </svg>
        )}
      </button>
      <button className="winbtn winbtn-close" aria-label={t("window.close")} title={t("window.close")} onClick={() => void act("close")}>
        <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
          <path d="M2.5 2.5l7 7M9.5 2.5l-7 7" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
        </svg>
      </button>
    </div>
  );
}
