import Select from "../../components/Select";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useGamePrefs } from "../../lib/prefs";
import { humanizeError } from "../../lib/format";
import { call, type GamePrefs } from "../../lib/api";
import { useEffect, useCallback } from "react";

type Preset = { id: string; patch: Partial<GamePrefs> };

/** Ready-made sets of options; they only fill in the form, nothing is written until "Apply". */
const PRESETS: Preset[] = [
  { id: "default", patch: { fps_max: 0, vsync: "default", show_fps: false, no_focus_sleep: false, high_priority: false, api: "default" } },
  { id: "smooth", patch: { fps_max: 144, vsync: "off", no_focus_sleep: true, high_priority: true, api: "default" } },
  { id: "weak", patch: { fps_max: 60, vsync: "on", no_focus_sleep: false, high_priority: false, api: "dx11" } },
];

function OwnSettings() {
  const { t } = useTranslation();
  const { prefs, setPrefs, save, dirty } = useGamePrefs();
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState(false);
  const [busy, setBusy] = useState(false);
  if (!prefs) return null;

  const put = (patch: Partial<GamePrefs>) => {
    setNote(false);
    setPrefs({ ...prefs, ...patch });
  };
  const fpsBad = prefs.fps_max !== 0 && (prefs.fps_max < 30 || prefs.fps_max > 1000);

  const apply = async () => {
    setBusy(true);
    try {
      await save(prefs);
      setError(null);
      setNote(true);
    } catch (e) {
      setError(humanizeError(t, e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <section className="block">
        <h2 className="block-title">{t("performance.presets")}</h2>
        <div className="row">
          {PRESETS.map((p) => (
            <button key={p.id} className="btn" onClick={() => put(p.patch)} title={t(`performance.preset_${p.id}_hint`)}>
              {t(`performance.preset_${p.id}`)}
            </button>
          ))}
        </div>
      </section>

      <section className="block">
        <h2 className="block-title">{t("performance.fps")}</h2>
        <label className="field-row">
          <span>{t("performance.fpsMax")}</span>
          <input
            className="field"
            type="number"
            min={0}
            max={1000}
            value={prefs.fps_max || ""}
            placeholder={t("performance.off")}
            aria-invalid={fpsBad}
            onChange={(e) => put({ fps_max: e.target.value === "" ? 0 : Math.round(Number(e.target.value)) })}
          />
        </label>
        {fpsBad && <p className="form-error">{t("performance.fpsRange")}</p>}
        <label className="field-row">
          <span>{t("performance.vsync")}</span>
          <Select
            label={t("performance.vsync")}
            value={prefs.vsync}
            onChange={(v) => put({ vsync: v as GamePrefs["vsync"] })}
            options={[
              { value: "default", label: t("performance.byGame") },
              { value: "on", label: t("performance.on") },
              { value: "off", label: t("performance.offVsync") },
            ]}
          />
        </label>
        <label className="check">
          <input type="checkbox" checked={prefs.show_fps} onChange={(e) => put({ show_fps: e.target.checked })} />
          <span>{t("performance.showFps")}</span>
        </label>
        <label className="check">
          <input type="checkbox" checked={prefs.no_focus_sleep} onChange={(e) => put({ no_focus_sleep: e.target.checked })} />
          <span>{t("performance.noFocusSleep")}</span>
        </label>
        <p className="note">{t("performance.autoexecNote")}</p>
      </section>

      <section className="block">
        <h2 className="block-title">{t("performance.launch")}</h2>
        <label className="field-row">
          <span>{t("performance.api")}</span>
          <Select
            label={t("performance.api")}
            value={prefs.api}
            onChange={(v) => put({ api: v as GamePrefs["api"] })}
            options={[
              { value: "default", label: t("performance.byGame") },
              { value: "dx11", label: "DirectX 11" },
              { value: "vulkan", label: "Vulkan" },
            ]}
          />
        </label>
        <label className="check">
          <input type="checkbox" checked={prefs.high_priority} onChange={(e) => put({ high_priority: e.target.checked })} />
          <span>{t("performance.high")}</span>
        </label>
        <p className="note">{t("performance.launchNote")}</p>
      </section>

      {error && <p className="form-error">{error}</p>}
      <div className="row">
        <button className="btn btn-primary" disabled={!dirty || busy || fpsBad} onClick={() => void apply()}>
          {t("performance.apply")}
        </button>
        {note && <span className="note">{t("performance.applied")}</span>}
      </div>
    </>
  );
}

type PerfPreset = { id: string; name: string; author: string; tier: string; blurb: string; recommended: boolean; url: string | null; license: string | null; settings: number; skipped: number };
type PerfStatus = { applied: string | null; settings: number; foreign: boolean };
type PerfState = { presets: PerfPreset[]; status: PerfStatus };

const TIERS = ["pretty", "balanced", "lean", "sweaty", "potato"] as const;

function Configs() {
  const { t } = useTranslation();
  const [st, setSt] = useState<PerfState | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [open, setOpen] = useState<string | null>(null);
  const [details, setDetails] = useState<[string, string][] | null>(null);
  const [tier, setTier] = useState<string>("all");
  const [askFix, setAskFix] = useState(false);

  const load = useCallback(async () => {
    try {
      setSt(await call<PerfState>("perf_state"));
      setError(null);
    } catch (e) {
      setError(humanizeError(t, e));
    }
  }, [t]);
  useEffect(() => {
    void load();
  }, [load]);

  const run = async (id: string | null) => {
    setBusy(true);
    try {
      await call("perf_apply", { id });
      await load();
    } catch (e) {
      setError(humanizeError(t, e));
    } finally {
      setBusy(false);
    }
  };

  // Takes the other manager's block out of gameinfo.gi (backed up, so the library's undo works too).
  const fix = async () => {
    setBusy(true);
    try {
      await call("perf_fix_foreign");
      setAskFix(false);
      await load();
    } catch (e) {
      setError(humanizeError(t, e));
    } finally {
      setBusy(false);
    }
  };

  const show = async (id: string) => {
    if (open === id) return setOpen(null);
    setOpen(id);
    setDetails(null);
    try {
      setDetails(await call<[string, string][]>("perf_details", { id }));
    } catch {
      setDetails([]);
    }
  };

  if (!st) return error ? <p className="form-error">{error}</p> : <p className="note">…</p>;
  const current = st.presets.find((p) => p.id === st.status.applied);
  const list = st.presets.filter((p) => tier === "all" || p.tier === tier).sort((a, b) => TIERS.indexOf(a.tier as never) - TIERS.indexOf(b.tier as never));

  return (
    <>
      {error && <p className="form-error">{error}</p>}
      {st.status.foreign && (
        <div className="notice" role="note">
          <b>{t("performance.foreignTitle")}</b>
          <span>{t("performance.foreignText")}</span>
          {askFix ? (
            <div className="notice-fix">
              <span>{t("performance.fixAsk")}</span>
              <div className="row">
                <button className="btn btn-primary" disabled={busy} onClick={() => void fix()}>
                  {t("performance.fixDo")}
                </button>
                <button className="btn btn-quiet" disabled={busy} onClick={() => setAskFix(false)}>
                  {t("performance.fixCancel")}
                </button>
              </div>
            </div>
          ) : (
            <div className="row">
              <button className="btn btn-primary" disabled={busy} onClick={() => setAskFix(true)}>
                {t("performance.fix")}
              </button>
            </div>
          )}
        </div>
      )}
      <div className="perf-current">
        <div>
          <small>{t("performance.current")}</small>
          <div className="perf-current-name">{current ? current.name : st.status.applied === "" ? t("performance.unknownConfig") : t("performance.none")}</div>
          <div className="slot-meta">{st.status.applied ? t("performance.settingsApplied", { count: st.status.settings }) : t("performance.noneHint")}</div>
        </div>
        {st.status.applied !== null && (
          <button className="btn" disabled={busy || st.status.foreign} onClick={() => void run(null)}>
            {t("performance.turnOff")}
          </button>
        )}
      </div>
      <p className="note">{t("performance.gameClosed")}</p>

      <div className="seg" role="radiogroup" aria-label={t("performance.tiers")}>
        {(["all", ...TIERS] as const).map((x) => (
          <button key={x} type="button" role="radio" aria-checked={tier === x} className={`seg-btn ${tier === x ? "seg-on" : ""}`} onClick={() => setTier(x)}>
            {x === "all" ? t("library.status_all") : t(`performance.tier_${x}`)}
          </button>
        ))}
      </div>
      {tier !== "all" && <p className="note">{t(`performance.tier_${tier}_hint`)}</p>}

      <ul className="perf-grid">
        {list.map((p) => {
          const applied = st.status.applied === p.id;
          return (
            <li key={p.id} className={`perf-card ${applied ? "perf-applied" : ""}`}>
              <div className="perf-top">
                <span className={`tier tier-${p.tier}`}>{t(`performance.tier_${p.tier}`)}</span>
                {p.recommended && <span className="tag tag-hero">{t("performance.recommended")}</span>}
              </div>
              <div className="perf-name">{p.name}</div>
              <div className="slot-meta">{t("performance.by", { author: p.author })}</div>
              <p className="perf-blurb">{p.blurb}</p>
              <div className="slot-meta">
                {t("performance.settingsCount", { count: p.settings })}
                {p.skipped > 0 && ` · ${t("performance.skipped", { count: p.skipped })}`}
              </div>
              <div className="perf-actions">
                <button className="btn btn-quiet" onClick={() => void show(p.id)} aria-expanded={open === p.id}>
                  {t("performance.details")}
                </button>
                <button className={`btn ${applied ? "" : "btn-primary"}`} disabled={busy || applied || st.status.foreign} onClick={() => void run(p.id)}>
                  {applied ? t("performance.applied") : t("performance.apply")}
                </button>
              </div>
              {open === p.id && (
                <div className="perf-details">
                  {p.license && <p className="note">{t("performance.license", { license: p.license })}</p>}
                  {!details ? (
                    "…"
                  ) : (
                    <ul className="paths">
                      {details.map(([k, v]) => (
                        <li key={k}>
                          {k} = {v}
                        </li>
                      ))}
                    </ul>
                  )}
                </div>
              )}
            </li>
          );
        })}
      </ul>
    </>
  );
}

export default function Performance() {
  const { t } = useTranslation();
  const [tab, setTab] = useState<"configs" | "own">("configs");
  return (
    <section className="view">
      <header className="view-head">
        <h1 className="view-title">{t("performance.title")}</h1>
        <p className="view-sub">{t("performance.subtitle")}</p>
      </header>
      <div className="seg" role="tablist">
        {(["configs", "own"] as const).map((x) => (
          <button key={x} type="button" role="tab" aria-selected={tab === x} className={`seg-btn ${tab === x ? "seg-on" : ""}`} onClick={() => setTab(x)}>
            {t(`performance.tab_${x}`)}
          </button>
        ))}
      </div>
      {tab === "configs" ? <Configs /> : <OwnSettings />}
    </section>
  );
}
