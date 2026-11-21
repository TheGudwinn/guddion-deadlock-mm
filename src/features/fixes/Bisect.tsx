import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { InstalledMod } from "../../lib/api";

type Props = {
  mods: InstalledMod[] | null;
  busy: boolean;
  gameFound: boolean;
  gameRunning: boolean;
  /** Switches mods on and off and resolves when the files have changed. */
  apply: (on: number[], off: number[]) => Promise<unknown>;
  onPlay: () => void;
};

type Run = {
  /** Mods that were on when the search started; put back at the end. */
  origin: number[];
  /** Mods that may still be the cause. */
  suspects: number[];
  /** Switched off for the current test. */
  off: number[];
  step: number;
};

/** Halves the suspects: the first half stays on, the second half is switched off for the test. */
const split = (s: number[]) => {
  const keep = Math.ceil(s.length / 2);
  return { kept: s.slice(0, keep), off: s.slice(keep) };
};

/**
 * Finds the mod that breaks the game by halving: switch half of the suspects off, the player starts the game
 * and says whether the problem is still there, and the search continues in the half that holds the cause.
 */
export default function Bisect({ mods, busy, gameFound, gameRunning, apply, onPlay }: Props) {
  const { t } = useTranslation();
  const [run, setRun] = useState<Run | null>(null);
  const [culprit, setCulprit] = useState<number | null>(null);
  const [working, setWorking] = useState(false);

  const own = (mods ?? []).filter((m) => m.enabled);
  const name = (id: number) => mods?.find((m) => m.id === id)?.name ?? String(id);
  const lock = busy || working;

  /** Brings the mods to the wanted state: `wantOn` enabled, `wantOff` disabled; untouched ones stay as they are. */
  const put = async (wantOn: number[], wantOff: number[]) => {
    const now = new Map((mods ?? []).map((m) => [m.id, m.enabled]));
    setWorking(true);
    try {
      await apply(
        wantOn.filter((id) => now.get(id) === false),
        wantOff.filter((id) => now.get(id) === true),
      );
    } finally {
      setWorking(false);
    }
  };

  const start = async () => {
    const ids = own.map((m) => m.id);
    const { off } = split(ids);
    setCulprit(null);
    setRun({ origin: ids, suspects: ids, off, step: 1 });
    await put(
      ids.filter((id) => !off.includes(id)),
      off,
    );
  };

  /** `still`: the problem is still there, so the cause is among the mods that stayed on. */
  const answer = async (still: boolean) => {
    if (!run) return;
    const { kept, off } = split(run.suspects);
    const next = still ? kept : off;
    if (next.length === 1) {
      setCulprit(next[0]);
      setRun({ ...run, suspects: next, off: [], step: run.step + 1 });
      await put(
        run.origin.filter((id) => id !== next[0]),
        [next[0]],
      );
      return;
    }
    const nextSplit = split(next);
    setRun({ ...run, suspects: next, off: nextSplit.off, step: run.step + 1 });
    await put(
      run.origin.filter((id) => !nextSplit.off.includes(id)),
      nextSplit.off,
    );
  };

  /** Puts every mod back as it was before the search. */
  const cancel = async () => {
    if (run) await put(run.origin, []);
    setRun(null);
    setCulprit(null);
  };

  const keepOff = () => {
    setRun(null);
    setCulprit(null);
  };

  const steps = run ? Math.max(1, Math.ceil(Math.log2(run.suspects.length))) : 0;

  return (
    <section className="block">
      <div className="view-title-row">
        <h2 className="block-title">{t("fixes.bisect.title")}</h2>
        {!run && (
          <button className="btn btn-primary" disabled={lock || own.length < 2} onClick={() => void start()}>
            {t("fixes.bisect.start")}
          </button>
        )}
      </div>
      <p className="note">{own.length < 2 && !run ? t("fixes.bisect.few") : t("fixes.bisect.text")}</p>

      {run && culprit === null && (
        <div className="notice" role="status">
          <b>{t("fixes.bisect.step", { step: run.step })}</b>
          <span>{t("fixes.bisect.testing", { on: run.suspects.length - run.off.length, off: run.off.length, left: run.suspects.length, steps })}</span>
          <ul className="find-items">
            {run.off.slice(0, 6).map((id) => (
              <li key={id}>{name(id)}</li>
            ))}
            {run.off.length > 6 && <li>{t("fixes.more", { count: run.off.length - 6 })}</li>}
          </ul>
          <div className="row">
            <button className="btn btn-primary" disabled={!gameFound || gameRunning || lock} onClick={onPlay}>
              {t("fixes.bisect.play")}
            </button>
          </div>
          {gameRunning && <span className="note">{t("fixes.bisect.running")}</span>}
          <div className="row">
            <button className="btn" disabled={lock} onClick={() => void answer(true)}>
              {t("fixes.bisect.still")}
            </button>
            <button className="btn" disabled={lock} onClick={() => void answer(false)}>
              {t("fixes.bisect.gone")}
            </button>
            <button className="btn btn-quiet" disabled={lock} onClick={() => void cancel()}>
              {t("fixes.bisect.cancel")}
            </button>
          </div>
        </div>
      )}

      {run && culprit !== null && (
        <div className="notice ok-notice" role="status">
          <b>{t("fixes.bisect.found", { name: name(culprit) })}</b>
          <span>{t("fixes.bisect.foundText")}</span>
          <div className="row">
            <button className="btn btn-primary" disabled={lock} onClick={keepOff}>
              {t("fixes.bisect.keepOff")}
            </button>
            <button className="btn" disabled={lock} onClick={() => void cancel()}>
              {t("fixes.bisect.restore")}
            </button>
          </div>
        </div>
      )}
    </section>
  );
}
