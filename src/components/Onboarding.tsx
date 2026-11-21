import { useTranslation } from "react-i18next";
import Icon from "./Icons";

type Props = {
  gameFound: boolean;
  hasMods: boolean;
  launched: boolean;
  onGoSettings: () => void;
  onGoCatalog: () => void;
  onPlay: () => void;
  onHide: () => void;
};

/** Three steps for a fresh install: find the game, add a mod, start. Shown on the home page until done or hidden. */
export default function Onboarding({ gameFound, hasMods, launched, onGoSettings, onGoCatalog, onPlay, onHide }: Props) {
  const { t } = useTranslation();
  const steps = [
    { done: gameFound, title: t("onboarding.game"), text: gameFound ? t("onboarding.gameOk") : t("onboarding.gameNo"), action: gameFound ? null : { label: t("onboarding.gamePick"), run: onGoSettings } },
    { done: hasMods, title: t("onboarding.mod"), text: t("onboarding.modText"), action: hasMods ? null : { label: t("onboarding.modGo"), run: onGoCatalog } },
    { done: launched, title: t("onboarding.play"), text: t("onboarding.playText"), action: launched || !gameFound ? null : { label: t("game.play"), run: onPlay } },
  ];
  const done = steps.filter((s) => s.done).length;
  const current = steps.findIndex((s) => !s.done);

  return (
    <section className="onboard" aria-label={t("onboarding.title")}>
      <header className="onboard-head">
        <div>
          <h2>{t("onboarding.title")}</h2>
          <p className="note">{t("onboarding.progress", { done, total: steps.length })}</p>
        </div>
        <button className="btn btn-quiet btn-sm" onClick={onHide}>
          {t("onboarding.hide")}
        </button>
      </header>
      <ol className="onboard-steps">
        {steps.map((s, i) => (
          <li key={s.title} className={`onboard-step ${s.done ? "onboard-done" : ""} ${i === current ? "onboard-now" : ""}`}>
            <span className="onboard-num" aria-hidden="true">
              {s.done ? <Icon name="check" size={16} /> : i + 1}
            </span>
            <div className="onboard-body">
              <b>{s.title}</b>
              <span>{s.text}</span>
            </div>
            {s.action && (
              <button className={"btn btn-sm"} onClick={s.action.run}>
                {s.action.label}
              </button>
            )}
          </li>
        ))}
      </ol>
    </section>
  );
}
