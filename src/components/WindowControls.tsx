import { useEffect } from "react";
import { useTranslation } from "react-i18next";

/** Minimise / close for the frameless window (no full-screen button on purpose). Rendered only inside the native app. */
export default function WindowControls({ onMaximizedChange }: { onMaximizedChange: (m: boolean) => void }) {
  const { t } = useTranslation();

  useEffect(() => {
    let off: (() => void) | undefined;
    let alive = true;
    void (async () => {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const win = getCurrentWindow();
      const sync = async () => {
        const m = await win.isMaximized();
        // No full-screen mode on purpose: double-click on the title area or Win+Up are undone at once.
        if (m) await win.unmaximize();
        if (alive) {
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

  const act = async (what: "minimize" | "close") => {
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
      <button className="winbtn winbtn-close" aria-label={t("window.close")} title={t("window.close")} onClick={() => void act("close")}>
        <svg width="12" height="12" viewBox="0 0 12 12" aria-hidden="true">
          <path d="M2.5 2.5l7 7M9.5 2.5l-7 7" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
        </svg>
      </button>
    </div>
  );
}
