import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { call, type AddonContent, type Finding, type InstalledMod, type UpdateReport } from "../../lib/api";
import Bisect from "./Bisect";
import { ackPatch, findSuspects, patchAcked } from "../../lib/patch";
import { humanizeError } from "../../lib/format";
import { isFixable, repairTitle, type Compat } from "../../lib/compat";

type Props = {
  game: unknown;
  compat: Compat;
  mods: InstalledMod[] | null;
  contents: AddonContent[];
  report: UpdateReport | null;
  checking: boolean;
  busy: boolean;
  onCheckUpdates: () => void;
  onUpdate: (id: number) => void;
  onUpdateMany: (ids: number[]) => void;
  /** Switches the mods off in one go (journaled, so the library undo button brings them back). */
  onDisable: (ids: number[]) => void;
  gameRunning: boolean;
  onPlay: () => void;
  /** Switches mods on and off in one go and resolves when done. */
  onApply: (on: number[], off: number[]) => Promise<unknown>;
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
      {done && (repairTitle(t, done) || done.skipped.length > 0) && (
        <div className="notice ok-notice" role="status">
          {repairTitle(t, done) && <b>{repairTitle(t, done)}</b>}
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

function PatchPanel({ game, mods, contents, report, checking, busy, onCheckUpdates, onUpdate, onUpdateMany, onDisable }: Omit<Props, "compat" | "onChanged" | "gameRunning" | "onPlay" | "onApply">) {
  const { t } = useTranslation();
  const build = (game as { build_id?: string | null } | null | undefined)?.build_id ?? null;
  const [acked, setAcked] = useState(() => patchAcked(build));
  const suspects = findSuspects(mods, contents, build, report);
  if (!build || suspects.length === 0) return null;
  const risky = suspects.filter((s) => s.risk !== "low");
  const withUpdate = suspects.filter((s) => s.update);

  return (
    <section className="block">
      <div className="view-title-row">
        <h2 className="block-title">{t("fixes.patch.title", { build })}</h2>
        <button className="btn" disabled={checking || busy} onClick={onCheckUpdates}>
          {checking ? t("fixes.patch.checking") : t("fixes.patch.check")}
        </button>
      </div>
      <p className="note">{t("fixes.patch.text", { count: suspects.length })}</p>
      <div className="row">
        {risky.length > 0 && (
          <button className="btn btn-primary" disabled={busy} onClick={() => onDisable(risky.map((s) => s.mod.id))}>
            {t("fixes.patch.disableRisky", { count: risky.length })}
          </button>
        )}
        {withUpdate.length > 0 && (
          <button className="btn" disabled={busy} onClick={() => onUpdateMany(withUpdate.map((s) => s.mod.id))}>
            {t("fixes.patch.updateAll", { count: withUpdate.length })}
          </button>
        )}
        {!acked && (
          <button
            className="btn btn-quiet"
            onClick={() => {
              ackPatch(build);
              setAcked(true);
            }}
          >
            {t("fixes.patch.dismiss")}
          </button>
        )}
      </div>
      <ul className="finds">
        {suspects.map((s) => (
          <li key={s.mod.id} className={`find find-${s.risk === "high" ? "error" : s.risk === "mid" ? "warn" : "info"}`}>
            <div className="find-main">
              <div className="find-title">{s.mod.name}</div>
              <p className="find-text">
                {t(`fixes.patch.risk_${s.risk}`)}
                {s.kinds.length > 0 && ` · ${s.kinds.map((k) => t(`kind.${k}`)).join(", ")}`}
                {` · ${t("fixes.patch.built", { build: s.mod.game_build })}`}
              </p>
            </div>
            {s.update && (
              <button className="btn btn-primary" disabled={busy} onClick={() => onUpdate(s.mod.id)}>
                {t("fixes.patch.update")}
              </button>
            )}
          </li>
        ))}
      </ul>
      <p className="note">{t("fixes.patch.note")}</p>
    </section>
  );
}

export default function Fixes({ game, compat, onChanged, gameRunning, onPlay, onApply, ...patch }: Props) {
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

      <PatchPanel game={game} {...patch} />

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
      <Bisect mods={patch.mods} busy={patch.busy} gameFound={!!game} gameRunning={gameRunning} apply={onApply} onPlay={onPlay} />
      <CompatPanel compat={compat} />
    </section>
  );
}
