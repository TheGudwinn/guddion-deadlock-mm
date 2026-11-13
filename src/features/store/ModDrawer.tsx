import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { call, isTauri, type Mod, type ModDetails } from "../../lib/api";
import { fileExt, formatBytes, htmlToText, humanizeError, isInstallable } from "../../lib/format";

type Props = {
  mod: Mod;
  installed: boolean;
  /** Already waiting in the download queue (or downloading / installing). */
  queued: boolean;
  busy: boolean;
  onClose: () => void;
  onInstall: (mod: Mod, fileId: string) => void;
};

async function openExternal(url: string) {
  if (isTauri) {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(url);
  } else {
    window.open(url, "_blank", "noopener");
  }
}

export default function ModDrawer({ mod, installed, queued, busy, onClose, onInstall }: Props) {
  const { t } = useTranslation();
  const [d, setD] = useState<ModDetails | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let stale = false;
    setD(null);
    setError(null);
    call<ModDetails>("mod_details", { id: mod.id }).then(
      (r) => !stale && setD(r),
      (e) => !stale && setError(humanizeError(t, e)),
    );
    return () => {
      stale = true;
    };
  }, [mod.id, t]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const image = d?.images[0] ?? mod.thumbnail;
  const desc = d?.description_html ? htmlToText(d.description_html) : "";

  return (
    <>
      <div className="scrim" onClick={onClose} />
      <aside className="drawer" role="dialog" aria-modal="true" aria-label={mod.name}>
        {image ? <img className="drawer-img" src={image} alt="" /> : <div className="drawer-img" />}
        <button className="btn drawer-close" onClick={onClose} autoFocus>
          {t("drawer.close")}
        </button>
        <div className="drawer-body">
          <h2>{mod.name}</h2>
          <p className="kv">{[mod.author, mod.category, mod.version && `v${mod.version}`].filter(Boolean).join(" · ")}</p>
          {d?.downloads != null && <p className="kv stat">{t("drawer.downloads", { count: d.downloads })}</p>}
          {desc && <p className="desc">{desc}</p>}
          <p className="kv">
            <button className="link" onClick={() => void openExternal(mod.page_url)}>
              {t("drawer.openPage")}
            </button>
          </p>

          <div className="section-label">{t("drawer.files")}</div>
          {error && <p className="note note-warn">{error}</p>}
          {!d && !error && <p className="note">…</p>}
          {d && d.files.length === 0 && <p className="note">{t("drawer.noFiles")}</p>}
          {d?.files.map((f) => {
            const ok = isInstallable(f.name);
            return (
              <div className="file" key={f.id}>
                <div className="file-info">
                  <div className="file-name">{f.name}</div>
                  <div className="file-sub">
                    {formatBytes(f.size)} · {f.av_clean ? t("drawer.avClean") : t("drawer.av")}
                  </div>
                  {!ok && <div className="file-sub">{t("drawer.unsupported", { ext: fileExt(f.name) || "?" })}</div>}
                </div>
                <button className="btn btn-primary" disabled={!ok || installed || queued || busy} onClick={() => onInstall(mod, f.id)}>
                  {installed ? t("drawer.installed") : queued ? t("drawer.queued") : t("drawer.install")}
                </button>
              </div>
            );
          })}
        </div>
      </aside>
    </>
  );
}
