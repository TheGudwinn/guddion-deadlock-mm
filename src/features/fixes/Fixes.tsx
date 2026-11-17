import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { call, type Finding } from "../../lib/api";
import { humanizeError } from "../../lib/format";
import { isFixable, type Compat } from "../../lib/compat";

type Props = {
  game: unknown;
  compat: Compat;
  /** Called after a fix changed game files, so the library and counters are read again. */
  onChanged: () => void;
};

const shortName = (n: string) => n.replace(/\.disabled$/i, "");

function CompatPanel({ compat }: { compat: Compat }) {
  const { t } = useTranslation();
  const { report: rep, scanning, repairing, error, done } = compat;
  const busy = scanning || repairing;

  const fixable = rep?.findings.some(isFixable) ?? false;

  return (
    <section className="block">
      <div className="view-title-row">
        <h2 className="block-title">{t("fixes.compat.title")}</h2>
        <button className="btn btn-primary" disabled={busy} onClick={() => void compat.scan()}>
          {busy ? t("fixes.compat.scanning") : t("fixes.compat.scan")}
        </button>
      </div>
      <p className="note">{t("fixes.compat.text")}</p>
      {error && <p className="form-error">{error}</p>}
      {done && (
        <div className="notice ok-notice" role="status">
          <b>{t("fixes.compat.repaired", { count: done.fixed.length })}</b>
          {done.skipped.map(([m, why]) => (
            <span key={m}>
              {m}: {why}
            </span>
          ))}
        </div>
      )}
      {rep && fixable && (
        <div className="row">
          <button className="btn btn-primary" disabled={busy} onClick={() => void compat.repair()}>
            {t("fixes.compat.repair")}
          </button>
          <span className="note">{t("fixes.compat.repairHint")}</span>
        </div>
      )}
      {rep && (
        <>
          {rep.findings.length === 0 && (
            <div className="notice ok-notice" role="status">
              <span>{t("fixes.compat.clean")}</span>
            </div>
          )}
          {rep.fixed.length > 0 && (
            <div className="notice ok-notice" role="status">
              <b>{t("fixes.compat.fixedMods", { count: rep.fixed.length })}</b>
              <span>{rep.fixed.map((x) => shortName(x.file)).join(" · ")}</span>
            </div>
          )}
          <ul className="finds">
            {rep.findings.map((f) => (
              <li key={f.kind + f.model} className={`find find-${f.severity}`}>
                <div className="find-main">
                  <div className="find-title">{t(`fixes.compat.${f.kind}`)}</div>
                  <p className="find-text">{t(`fixes.compat.${f.kind}_text`)}</p>
                  <ul className="find-items">
                    <li>{f.model}</li>
                    {f.mods.length > 0 && <li>{f.mods.map(shortName).join(" · ")}</li>}
                  </ul>
                </div>
              </li>
            ))}
          </ul>
          <details className="compat-details">
            <summary>{t("fixes.compat.details")}</summary>
            <ul className="find-items">
              <li>{t("fixes.compat.mods", { count: rep.details.mods_scanned })}</li>
              <li>{t("fixes.compat.heroModels", { count: rep.details.hero_models, read: rep.details.models_looked_into })}</li>
              {rep.details.mods_unreadable.length > 0 && <li>{t("fixes.compat.unreadable", { list: rep.details.mods_unreadable.join(", ") })}</li>}
              {rep.details.skipped.length > 0 && <li>{t("fixes.compat.skipped")}</li>}
              {rep.details.skipped.map(([m, p, why]) => (
                <li key={m + p}>
                  {shortName(m)} · {p} · {why}
                </li>
              ))}
            </ul>
          </details>
          <p className="note">{t("fixes.compat.repairNote")}</p>
        </>
      )}
    </section>
  );
}

export default function Fixes({ game, compat, onChanged }: Props) {
  const { t } = useTranslation();
  const [findings, setFindings] = useState<Finding[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [working, setWorking] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);

  const scan = useCallback(async () => {
    setError(null);
    try {
      setFindings(await call<Finding[]>("run_checks"));
    } catch (e) {
      setFindings(null);
      setError(humanizeError(t, e));
    }
  }, [t]);

  useEffect(() => {
    void scan();
  }, [scan, game]);

  const fix = async (f: Finding) => {
    if (!f.fix) return;
    setWorking(f.kind);
    setNote(null);
    try {
      await call("apply_fix", { fix: f.fix, items: f.items });
      setNote(t("fixes.done"));
      onChanged();
      await scan();
    } catch (e) {
      setError(humanizeError(t, e));
    } finally {
      setWorking(null);
    }
  };

  return (
    <section className="view">
      <header className="view-head">
        <div className="view-title-row">
          <h1 className="view-title">{t("fixes.title")}</h1>
          <div className="head-actions">
            <button className="btn" disabled={!!working} onClick={() => void scan()}>
              {t("fixes.recheck")}
            </button>
          </div>
        </div>
        <p className="view-sub">{t("fixes.subtitle")}</p>
      </header>

      {error && <p className="form-error">{error}</p>}
      {note && <p className="note">{note}</p>}
      {!findings && !error && <p className="note">…</p>}
      {findings && findings.length === 0 && (
        <div className="notice ok-notice" role="status">
          <b>{t("fixes.allGood")}</b>
          <span>{t("fixes.allGoodText")}</span>
        </div>
      )}

      <ul className="finds">
        {findings?.map((f) => (
          <li key={f.kind} className={`find find-${f.severity}`}>
            <div className="find-main">
              <div className="find-title">{t(`fixes.${f.kind}.title`)}</div>
              <p className="find-text">{t(`fixes.${f.kind}.text`)}</p>
              {f.items.length > 0 && (
                <ul className="find-items">
                  {f.items.slice(0, 8).map((i) => (
                    <li key={i}>{shortName(i)}</li>
                  ))}
                  {f.items.length > 8 && <li>{t("fixes.more", { count: f.items.length - 8 })}</li>}
                </ul>
              )}
            </div>
            {f.fix && (
              <button className="btn btn-primary" disabled={!!working} onClick={() => void fix(f)}>
                {working === f.kind ? "…" : t(`fixes.fix_${f.fix}`)}
              </button>
            )}
          </li>
        ))}
      </ul>
      <p className="note">{t("fixes.undoNote")}</p>
      <CompatPanel compat={compat} />
    </section>
  );
}
