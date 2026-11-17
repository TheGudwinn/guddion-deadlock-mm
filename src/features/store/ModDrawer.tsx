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
  const [shot, setShot] = useState(0); // screenshot shown in the header
  const [zoom, setZoom] = useState(false); // full-size viewer

  useEffect(() => {
    let stale = false;
    setD(null);
    setError(null);
    setShot(0);
    setZoom(false);
    call<ModDetails>("mod_details", { id: mod.id }).then(
      (r) => !stale && setD(r),
      (e) => !stale && setError(humanizeError(t, e)),
    );
    return () => {
      stale = true;
    };
  }, [mod.id, t]);

  const shots = d?.images.length ? d.images : mod.thumbnail ? [mod.thumbnail] : [];
  const step = (by: number) => setShot((i) => (shots.length ? (i + by + shots.length) % shots.length : 0));

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        // The viewer closes first, then the drawer.
        if (zoom) setZoom(false);
        else onClose();
      } else if (shots.length > 1 && (e.key === "ArrowRight" || e.key === "ArrowLeft")) {
        step(e.key === "ArrowRight" ? 1 : -1);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- step only depends on shots.length
  }, [onClose, zoom, shots.length]);

  const image = shots[Math.min(shot, Math.max(0, shots.length - 1))] ?? null;
  const desc = d?.description_html ? htmlToText(d.description_html) : "";

  return (
    <>
      <div className="scrim" onClick={onClose} />
      <aside className="drawer" role="dialog" aria-modal="true" aria-label={mod.name}>
        {image ? (
          <button type="button" className="drawer-img-btn" onClick={() => setZoom(true)} aria-label={t("drawer.openShot")} title={t("drawer.openShot")}>
            <img className="drawer-img" src={image} alt="" />
          </button>
        ) : (
          <div className="drawer-img" />
        )}
        <button className="btn drawer-close" onClick={onClose} autoFocus>
          {t("drawer.close")}
        </button>
        <div className="drawer-body">
          <h2>{mod.name}</h2>
          {shots.length > 1 && (
            <div className="shots" role="list" aria-label={t("drawer.screens", { count: shots.length })}>
              {shots.map((u, i) => (
                <button key={u} type="button" role="listitem" className={`shot ${i === shot ? "shot-on" : ""}`} aria-label={`${t("drawer.screen")} ${i + 1}`} onClick={() => setShot(i)}>
                  <img src={u} alt="" loading="lazy" decoding="async" />
                </button>
              ))}
            </div>
          )}
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
      {zoom && image && (
        <div className="lightbox" role="dialog" aria-modal="true" aria-label={mod.name} onClick={() => setZoom(false)}>
          <img src={image} alt="" onClick={(e) => e.stopPropagation()} />
          {shots.length > 1 && (
            <>
              <button
                className="lb-nav lb-prev"
                aria-label={t("drawer.prev")}
                onClick={(e) => {
                  e.stopPropagation();
                  step(-1);
                }}
              >
                ‹
              </button>
              <button
                className="lb-nav lb-next"
                aria-label={t("drawer.next")}
                onClick={(e) => {
                  e.stopPropagation();
                  step(1);
                }}
              >
                ›
              </button>
              <span className="lb-count">
                {Math.min(shot, shots.length - 1) + 1} / {shots.length}
              </span>
            </>
          )}
          <button className="btn lb-close" onClick={() => setZoom(false)}>
            {t("drawer.close")}
          </button>
        </div>
      )}
    </>
  );
}
