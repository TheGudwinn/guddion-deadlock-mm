import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useGamePrefs } from "../../lib/prefs";
import { humanizeError } from "../../lib/format";
import Select from "../../components/Select";

/** Launch options block of the settings page: what the Play button starts the game with. */
export default function GameOptions() {
  const { t } = useTranslation();
  const { prefs, setPrefs, save, dirty } = useGamePrefs();
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState(false);
  if (!prefs) return null;

  const put = (patch: Partial<typeof prefs>) => {
    setNote(false);
    setPrefs({ ...prefs, ...patch });
  };

  return (
    <section className="block">
      <h2 className="block-title">{t("settings.launch.title")}</h2>
      <p className="note">{t("settings.launch.hint")}</p>
      <label className="check">
        <input type="checkbox" checked={prefs.novid} onChange={(e) => put({ novid: e.target.checked })} />
        <span>{t("settings.launch.novid")}</span>
      </label>
      <label className="check">
        <input type="checkbox" checked={prefs.console} onChange={(e) => put({ console: e.target.checked })} />
        <span>{t("settings.launch.console")}</span>
      </label>
      <div className="row">
        <Select
          label={t("settings.launch.window")}
          value={prefs.window}
          onChange={(v) => put({ window: v as typeof prefs.window })}
          options={[
            { value: "default", label: t("settings.launch.windowDefault") },
            { value: "windowed", label: t("settings.launch.windowed") },
            { value: "fullscreen", label: t("settings.launch.fullscreen") },
          ]}
        />
      </div>
      <input className="field mono" value={prefs.extra} placeholder={t("settings.launch.extra")} aria-label={t("settings.launch.extra")} onChange={(e) => put({ extra: e.target.value })} />
      {error && <p className="form-error">{error}</p>}
      <div className="row">
        <button
          className="btn btn-primary"
          disabled={!dirty}
          onClick={() =>
            void save(prefs).then(
              () => {
                setError(null);
                setNote(true);
              },
              (e) => setError(humanizeError(t, e)),
            )
          }
        >
          {t("settings.save")}
        </button>
        {note && <span className="note">{t("settings.saved")}</span>}
      </div>
    </section>
  );
}
