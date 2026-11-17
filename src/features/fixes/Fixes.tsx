import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { call, type Finding } from "../../lib/api";
import { humanizeError } from "../../lib/format";

type Props = {
  game: unknown;
  /** Called after a fix changed game files, so the library and counters are read again. */
  onChanged: () => void;
};

const shortName = (n: string) => n.replace(/\.disabled$/i, "");

type CompatFinding = { kind: string; severity: string; model: string; mods: string[] };
type CompatReport = {
  findings: CompatFinding[];
  details: { mods_scanned: number; mods_unreadable: string[]; hero_models: number; models_looked_into: number; skipped: [string, string, string][] };
};

function CompatPanel() {
  const { t } = useTranslation();
  const [rep, setRep] = useState<CompatReport | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [done, setDone] = useState<{ fixed: string[]; skipped: [string, string][] } | null>(null);

  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      setRep(await call<CompatReport>("scan_compat"));
    } catch (e) {
      setError(humanizeError(t, e));
    } finally {
      setBusy(false);
    }
  };

  const repair = async () => {
    setBusy(true);
    setError(null);
    try {
      setDone(await call<{ fixed: string[]; skipped: [string, string][] }>("repair_cameras"));
      setRep(await call<CompatReport>("scan_compat"));
    } catch (e) {
      setError(humanizeError(t, e));
    } finally {
      setBusy(false);
    }
  };

  const fixable = rep?.findings.some((f) => f.kind === "camera_interface_missing") ?? false;

  return (
    <section className="block">
      <div className="view-title-row">
        <h2 className="block-title">{t("fixes.compat.title")}</h2>
        <button className="btn btn-primary" disabled={busy} onClick={() => void run()}>
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
          <button className="btn btn-primary" disabled={busy} onClick={() => void repair()}>
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
          <ul className="finds">
            {rep.findings.map((f) => (
              <li key={f.kind + f.model} className={`find find-${f.severity}`}>
                <div className="find-main">
                  <div className="find-title">{t(`fixes.compat.${f.kind}`)}</div>
                  <p className="find-text">{t(`fixes.compat.${f.kind}_text`)}</p>
                  <ul className="find-items">
                    <li>{f.model}</li>
                    <li>{f.mods.map(shortName).join(" · ")}</li>
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

export default function Fixes({ game, onChanged }: Props) {
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
      <CompatPanel />
    </section>
  );
}
