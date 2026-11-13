import { useTranslation } from "react-i18next";
import type { UpdateInfo } from "../lib/api";

type Props = {
  info: UpdateInfo;
  /** null = not downloading; otherwise percent (or -1 when the size is unknown). */
  progress: number | null;
  onInstall: () => void;
  onDismiss: () => void;
};

export default function UpdateBanner({ info, progress, onInstall, onDismiss }: Props) {
  const { t } = useTranslation();
  const busy = progress !== null;
  return (
    <div className="update" role="status">
      <div className="update-text">
        <b>{t("update.available", { version: info.version })}</b>
        <span>{busy ? t("update.installing") : t("update.hint", { current: info.current })}</span>
        {busy && (
          <div className="bar" aria-hidden="true">
            <i style={{ width: progress >= 0 ? `${progress}%` : "100%" }} className={progress < 0 ? "bar-indeterminate" : ""} />
          </div>
        )}
      </div>
      <div className="update-actions">
        <button className="btn btn-primary" disabled={busy} onClick={onInstall}>
          {t("update.install")}
        </button>
        {!busy && (
          <button className="btn btn-quiet" onClick={onDismiss}>
            {t("update.later")}
          </button>
        )}
      </div>
    </div>
  );
}
