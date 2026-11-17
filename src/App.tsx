import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { flushSync } from "react-dom";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { useForeignMods, type ForeignGroup } from "./lib/foreign";
import {
  call,
  isTauri,
  type ApplyResult,
  type Profile,
  type AddonContent,
  type ForeignAddon,
  type GameInstall,
  type InstallResult,
  type InstalledMod,
  type LastAction,
  type Mod,
  type ProgressEvent,
  type QueueRow,
  type UpdateInfo,
  type UpdateReport,
} from "./lib/api";
import { humanizeError } from "./lib/format";
import { useCompat } from "./lib/compat";
import Activity, { type ActivityState } from "./components/Activity";
import UpdateBanner from "./components/UpdateBanner";
import WindowControls from "./components/WindowControls";
import Icon from "./components/Icons";
import Select from "./components/Select";
import Confirm, { type ConfirmState } from "./components/Confirm";
import History, { type HistoryItem } from "./components/History";
import { setAppearance, useAppearance, useWallpaper } from "./lib/appearance";
import Home from "./features/home/Home";
import logo from "./assets/logo.png";
import Catalog from "./features/store/Catalog";
import ModDrawer from "./features/store/ModDrawer";
import Library from "./features/library/Library";
import Profiles from "./features/profiles/Profiles";
import Settings from "./features/settings/Settings";
import Downloads from "./features/downloads/Downloads";
import Performance from "./features/performance/Performance";
import Fixes from "./features/fixes/Fixes";

type View = "home" | "catalog" | "library" | "profiles" | "downloads" | "performance" | "fixes" | "settings";

export default function App() {
  const { t } = useTranslation();
  const look = useAppearance();
  const wallpaper = useWallpaper();
  const [view, setViewRaw] = useState<View>("home");
  const viewRef = useRef(view);
  viewRef.current = view;
  // Page changes cross-fade through the browser's View Transitions when it has them.
  const setView = useCallback((v: View) => {
    if (v === viewRef.current) return;
    const doc = document as Document & { startViewTransition?: (cb: () => void) => unknown };
    const still = document.documentElement.dataset.motion === "off" || window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    if (doc.startViewTransition && !still && !document.hidden) {
      doc.startViewTransition(() => flushSync(() => setViewRaw(v)));
      // a window that is not being drawn never runs the transition callback; do not leave the page stuck
      window.setTimeout(() => {
        if (viewRef.current !== v) setViewRaw(v);
      }, 220);
    } else setViewRaw(v);
  }, []);
  const railRef = useRef<HTMLElement>(null);
  const [ind, setInd] = useState<{ y: number; h: number } | null>(null);
  const [indReady, setIndReady] = useState(false);
  const [searchText, setSearchText] = useState("");
  const [search, setSearch] = useState({ text: "", n: 0 });
  const [preset, setPreset] = useState<{ category: number | null; sub: number | null; n: number } | undefined>(undefined);
  const [game, setGame] = useState<GameInstall | null | undefined>(undefined);
  const [installed, setInstalled] = useState<InstalledMod[] | null>(null);
  const [foreign, setForeign] = useState<ForeignAddon[]>([]);
  const [contents, setContents] = useState<AddonContent[]>([]);
  const ownRemoteIds = useMemo(() => (installed ?? []).map((m) => m.remote_id).filter((id): id is string => !!id), [installed]);
  const { groups, known } = useForeignMods(foreign, ownRemoteIds);
  const [launching, setLaunching] = useState(false);
  const [last, setLast] = useState<LastAction | null>(null);
  const [profiles, setProfiles] = useState<Profile[] | null>(null);
  const [open, setOpen] = useState<Mod | null>(null);
  const [busy, setBusy] = useState(false);
  const [activity, setActivity] = useState<ActivityState | null>(null);
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [maximized, setMaximized] = useState(false);
  const [queue, setQueue] = useState<QueueRow[]>([]);
  const prevStates = useRef<Map<number, string>>(new Map());
  const compat = useCompat();
  const [modReport, setModReport] = useState<UpdateReport | null>(null);
  const [checkingMods, setCheckingMods] = useState(false);
  const autoChecked = useRef(false);
  const [updatePct, setUpdatePct] = useState<number | null>(null);
  const [running, setRunning] = useState(false);
  // something changed in the mods since the game was last started
  const [pending, setPending] = useState(false);
  const [confirm, setConfirm] = useState<ConfirmState | null>(null);
  const [history, setHistory] = useState<HistoryItem[]>([]);
  const [unread, setUnread] = useState(0);
  const [libSearch, setLibSearch] = useState({ text: "", n: 0 });
  const searchBox = useRef<HTMLInputElement>(null);
  const [launched, setLaunched] = useState(() => {
    try {
      return localStorage.getItem("guddion.launched") === "1";
    } catch {
      return false;
    }
  });
  const [onboardHidden, setOnboardHidden] = useState(() => {
    try {
      return localStorage.getItem("guddion.onboarding.hidden") === "1";
    } catch {
      return false;
    }
  });
  const markChanged = useCallback(() => setPending(true), []);

  // Is the game running? Asked every few seconds; changes made meanwhile apply on the next start.
  useEffect(() => {
    if (!isTauri) return;
    let alive = true;
    const poll = () => void call<boolean>("game_running").then((r) => alive && setRunning(r), () => {});
    poll();
    const id = setInterval(poll, 4000);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, []);
  const wasRunning = useRef(false);
  useEffect(() => {
    // a fresh start picks up every change made before it
    if (running && !wasRunning.current) setPending(false);
    wasRunning.current = running;
  }, [running]);

  // Toasts vanish; the bell keeps the last ones.
  const historyId = useRef(0);
  useEffect(() => {
    if (!activity || activity.kind === "progress") return;
    const item: HistoryItem = { id: ++historyId.current, at: Date.now(), kind: activity.kind, text: activity.text, sub: activity.kind === "ok" ? activity.sub : undefined };
    setHistory((h) => [...h.slice(-29), item]);
    setUnread((n) => n + 1);
  }, [activity]);

  const refresh = useCallback(async () => {
    const [mods, f, c, l, pr] = await Promise.all([
      call<InstalledMod[]>("list_installed").catch(() => null),
      call<ForeignAddon[]>("list_foreign_addons").catch(() => []),
      call<AddonContent[]>("describe_installed").catch(() => []),
      call<LastAction | null>("last_action").catch(() => null),
      call<Profile[]>("list_profiles").catch(() => null),
    ]);
    setProfiles(pr);
    setInstalled(mods);
    setForeign(f);
    setContents(c);
    setLast(l);
  }, []);

  const detectGame = useCallback(() => call<GameInstall | null>("detect_game").then(setGame, () => setGame(null)), []);

  useEffect(() => {
    void detectGame();
    void refresh();
  }, [detectGame, refresh]);

  // Startup check: silent on failure (offline, blocked GitHub, no source configured).
  useEffect(() => {
    void (async () => {
      try {
        const s = await call<{ check_updates: boolean }>("get_settings");
        if (!s.check_updates) return;
        setUpdate(await call<UpdateInfo | null>("check_update"));
      } catch {
        /* the settings screen has a manual check that reports errors */
      }
    })();
  }, []);

  const checkUpdate = async () => {
    const info = await call<UpdateInfo | null>("check_update");
    setUpdate(info);
    return info;
  };

  async function installUpdate() {
    setUpdatePct(-1);
    const unlisten = isTauri
      ? await listen<{ downloaded: number; total: number | null }>("update-progress", (ev) => {
          const { downloaded, total } = ev.payload;
          setUpdatePct(total ? Math.min(100, Math.round((downloaded / total) * 100)) : -1);
        })
      : () => {};
    try {
      await call("install_update"); // the app restarts on success
    } catch (e) {
      setActivity({ kind: "error", text: humanizeError(t, e) });
      setUpdatePct(null);
    } finally {
      unlisten();
    }
  }

  const checkMods = useCallback(async (manual: boolean) => {
    setCheckingMods(true);
    try {
      setModReport(await call<UpdateReport>("check_mod_updates"));
    } catch (e) {
      if (manual) setActivity({ kind: "error", text: humanizeError(t, e) });
    } finally {
      setCheckingMods(false);
    }
  }, [t]);

  // Once per launch, as soon as we know there is something installed.
  useEffect(() => {
    if (installed && installed.length > 0 && !autoChecked.current) {
      autoChecked.current = true;
      void checkMods(false);
    }
  }, [installed, checkMods]);

  async function updateMod(id: number) {
    const name = installed?.find((m) => m.id === id)?.name ?? "";
    setActivity({ kind: "progress", text: t("activity.downloading", { name }), pct: null });
    const unlisten = isTauri
      ? await listen<ProgressEvent>("install-progress", (ev) => {
          const p = ev.payload;
          if (p.stage === "install") setActivity({ kind: "progress", text: t("activity.installing", { name }), pct: null });
          else setActivity({ kind: "progress", text: t("activity.downloading", { name }), pct: p.total ? Math.round((p.downloaded / p.total) * 100) : null });
        })
      : () => {};
    const res = await run(() => call<InstallResult>("update_mod", { id }));
    unlisten();
    if (res) {
      setActivity({ kind: "ok", text: t("library.updated", { name: res.name }), sub: res.overlaps.length > 0 ? t("activity.overlaps", { count: new Set(res.overlaps.map((o) => o.with_addon)).size }) : undefined });
      setModReport((r) => (r ? { ...r, updates: r.updates.filter((u) => u.mod_id !== id) } : r));
      markChanged();
    } else {
      setActivity((a) => (a?.kind === "progress" ? null : a));
    }
  }

  // The queue lives in the backend; we reload it on every change event and patch progress in place.
  const loadQueue = useCallback(async () => {
    const rows = await call<QueueRow[]>("list_queue").catch(() => null);
    if (!rows) return;
    for (const r of rows) {
      const before = prevStates.current.get(r.id);
      if (!before || before === r.state) continue;
      if (r.state === "done") {
        setActivity({
          kind: "ok",
          text: t("activity.done", { name: r.name, slot: String(r.slot ?? 0).padStart(2, "0") }),
          sub: (r.overlaps ?? 0) > 0 ? t("activity.overlaps", { count: r.overlaps }) : undefined,
        });
        void refresh();
        markChanged();
      } else if (r.state === "failed") {
        setActivity({ kind: "error", text: `${r.name}: ${humanizeError(t, r.error ?? "")}` });
      }
    }
    prevStates.current = new Map(rows.map((r) => [r.id, r.state]));
    setQueue(rows);
  }, [t, refresh, markChanged]);

  useEffect(() => {
    void loadQueue();
    if (!isTauri) return;
    const subs = [
      listen("queue-changed", () => void loadQueue()),
      listen<{ id: number; downloaded: number; total: number | null }>("queue-progress", (ev) => {
        const { id, downloaded, total } = ev.payload;
        setQueue((rows) => rows.map((r) => (r.id === id ? { ...r, downloaded, total: total ?? r.total } : r)));
      }),
    ];
    return () => subs.forEach((p) => void p.then((off) => off()));
  }, [loadQueue]);

  const queueCall = (cmd: string, id?: number) =>
    void run(async () => {
      await call(cmd, id === undefined ? undefined : { id });
      await loadQueue();
    });
  const activeDownloads = queue.filter((r) => ["queued", "downloading", "paused", "installing"].includes(r.state));
  const queuedModIds = useMemo(() => new Set(activeDownloads.map((r) => r.remote_id)), [activeDownloads]);

  const installedIds = useMemo(() => new Set((installed ?? []).map((m) => m.remote_id).filter((x): x is string => !!x)), [installed]);

  async function run<T>(fn: () => Promise<T>): Promise<T | undefined> {
    setBusy(true);
    try {
      return await fn();
    } catch (e) {
      setActivity({ kind: "error", text: humanizeError(t, e) });
      return undefined;
    } finally {
      setBusy(false);
      void refresh();
    }
  }

  async function install(mod: Mod, fileId: string) {
    const ok = await run(async () => {
      await call("enqueue_install", { modId: mod.id, fileId });
      return true;
    });
    if (ok) {
      setActivity({ kind: "ok", text: t("downloads.added", { name: mod.name }) });
      setOpen(null);
      void loadQueue();
    }
  }

  const say = (text: string, sub?: string) => setActivity({ kind: "ok", text, sub });

  async function createProfile(name: string): Promise<boolean> {
    const done = await run(async () => {
      await call("create_profile", { name });
      return true;
    });
    if (done) say(t("profiles.created", { name: name.trim() }));
    return !!done;
  }

  async function renameProfile(id: number, name: string): Promise<boolean> {
    return !!(await run(async () => {
      await call("rename_profile", { id, name });
      return true;
    }));
  }

  const doApplyProfile = (p: Profile) =>
    void run(async () => {
      const r = await call<ApplyResult>("apply_profile", { id: p.id });
      markChanged();
      say(t("profiles.applied", { name: p.name, on: r.enabled, off: r.disabled }), r.missing.length > 0 ? t("profiles.appliedMissing", { count: r.missing.length }) : undefined);
    });
  const applyProfile = (p: Profile) => {
    const on = p.mods.filter((m) => m.installed && m.enabled).length;
    const missing = p.mods.filter((m) => !m.installed).length;
    setConfirm({
      title: t("confirm.applyTitle", { name: p.name }),
      text: t("confirm.applyText"),
      details: [t("confirm.applyOn", { count: on }), t("confirm.applyOff"), ...(missing > 0 ? [t("confirm.applyMissing", { count: missing })] : [])],
      confirm: t("profiles.apply"),
      onConfirm: () => doApplyProfile(p),
    });
  };

  const updateProfile = (p: Profile) =>
    void run(async () => {
      await call("update_profile_from_current", { id: p.id });
      say(t("profiles.updated", { name: p.name }));
    });

  const deleteProfile = (id: number) => {
    const p = profiles?.find((x) => x.id === id);
    setConfirm({ title: t("confirm.deleteProfileTitle", { name: p?.name ?? "" }), text: t("confirm.deleteProfileText"), confirm: t("profiles.removeYes"), danger: true, onConfirm: () => void run(() => call("delete_profile", { id })) });
  };

  const exportProfile = (p: Profile) =>
    void run(async () => {
      let path: string | null = "mock.json";
      if (isTauri) {
        const { save } = await import("@tauri-apps/plugin-dialog");
        path = await save({ title: t("profiles.exportTitle"), defaultPath: `${p.name}.json`, filters: [{ name: "Profile", extensions: ["json"] }] });
      }
      if (!path) return;
      await call("export_profile", { id: p.id, path });
      say(t("profiles.exported"));
    });

  const importProfile = () =>
    void run(async () => {
      let path: string | null = "mock.json";
      if (isTauri) {
        const { open } = await import("@tauri-apps/plugin-dialog");
        const picked = await open({ multiple: false, filters: [{ name: "Profile", extensions: ["json"] }] });
        path = typeof picked === "string" ? picked : null;
      }
      if (!path) return;
      const id = await call<number>("import_profile", { path });
      const list = await call<Profile[]>("list_profiles");
      say(t("profiles.imported", { name: list.find((p) => p.id === id)?.name ?? "" }));
    });

  const toggle = (id: number, enabled: boolean) =>
    void run(async () => {
      await call("set_mod_enabled", { id, enabled });
      markChanged();
    });
  const toggleForeign = (g: ForeignGroup, enabled: boolean) =>
    void run(async () => {
      await call("set_foreign_enabled", { files: g.files.map((f) => f.name), id: g.id, enabled });
      markChanged();
    });
  // Several mods at once: one request at a time, one summary at the end.
  const bulkToggle = (ids: number[], gs: ForeignGroup[], enabled: boolean) =>
    void run(async () => {
      for (const id of ids) await call("set_mod_enabled", { id, enabled });
      for (const g of gs) await call("set_foreign_enabled", { files: g.files.map((f) => f.name), id: g.id, enabled });
      markChanged();
      say(t(enabled ? "library.bulkOn" : "library.bulkOff", { count: ids.length + gs.length }));
    });
  const bulkRemove = (ids: number[]) => {
    const names = (installed ?? []).filter((m) => ids.includes(m.id)).map((m) => m.name);
    setConfirm({
      title: t("confirm.removeManyTitle", { count: ids.length }),
      text: t("confirm.removeText"),
      details: names.slice(0, 8).concat(names.length > 8 ? [t("confirm.andMore", { count: names.length - 8 })] : []),
      confirm: t("library.removeYes"),
      danger: true,
      onConfirm: () =>
        void run(async () => {
          for (const id of ids) await call("uninstall_mod", { id });
          markChanged();
        }),
    });
  };
  const launchGame = async () => {
    setLaunching(true);
    try {
      const args = await call<string>("launch_game");
      setPending(false);
      setLaunched(true);
      try {
        localStorage.setItem("guddion.launched", "1");
      } catch {
        /* only the first-run hint depends on it */
      }
      setActivity({ kind: "ok", text: args ? t("game.started", { args }) : t("game.startedPlain") });
    } catch (e) {
      setActivity({ kind: "error", text: humanizeError(t, e) });
    } finally {
      setTimeout(() => setLaunching(false), 4000); // Steam needs a moment; stops double clicks
    }
  };
  const remove = (id: number) => {
    const m = installed?.find((x) => x.id === id);
    setConfirm({
      title: t("confirm.removeTitle", { name: m?.name ?? "" }),
      text: t("confirm.removeText"),
      confirm: t("library.removeYes"),
      danger: true,
      onConfirm: () =>
        void run(async () => {
          await call("uninstall_mod", { id });
          markChanged();
        }),
    });
  };
  const reorder = (ids: number[]) =>
    void run(async () => {
      await call("reorder_mods", { order: ids });
      markChanged();
      say(t("library.reordered"));
    });
  const undo = () =>
    void run(async () => {
      await call("undo_last");
      markChanged();
      setActivity({ kind: "ok", text: t("library.undone") });
    });

  useLayoutEffect(() => {
    const place = () => {
      const el = railRef.current?.querySelector<HTMLElement>('[aria-current="page"]');
      setInd(el ? { y: el.offsetTop, h: el.offsetHeight } : null);
      if (el) requestAnimationFrame(() => requestAnimationFrame(() => setIndReady(true)));
    };
    place();
    window.addEventListener("resize", place);
    return () => window.removeEventListener("resize", place);
  }, [view, look.rail]);
  const order: View[] = ["home", "catalog", "profiles", "library", "downloads", "performance", "fixes", "settings"];
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        searchBox.current?.focus();
        searchBox.current?.select();
      } else if (e.altKey && !e.ctrlKey && /^[1-8]$/.test(e.key)) {
        e.preventDefault();
        setView(order[Number(e.key) - 1]);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- `order` is a constant list
  }, []);
  const hint = (id: View) => `${t(`nav.${id}`)}  ·  Alt+${order.indexOf(id) + 1}`;
  const activeProfile = profiles?.find((x) => x.active);
  const current = activeDownloads.find((r) => r.state === "downloading") ?? activeDownloads[0];
  const currentPct = current?.total ? Math.min(100, Math.round((current.downloaded / current.total) * 100)) : null;

  return (
    <div className={`shell ${maximized ? "maximized" : ""}`}>
      {wallpaper && <div className="wallpaper" aria-hidden="true" style={{ backgroundImage: `url(${wallpaper})` }} />}
      <div className="aurora" aria-hidden="true">
        <i />
        <i />
        <i />
        <b />
      </div>
      <nav ref={railRef} className={`rail ${look.rail === "open" ? "rail-open" : "rail-closed"}`} aria-label={t("app.name")}>
        <span className={`rail-ind ${indReady ? "rail-ind-ready" : ""}`} aria-hidden="true" style={ind ? { transform: `translateY(${ind.y}px)`, height: ind.h, opacity: 1 } : { opacity: 0 }} />
        <div className="rail-logo" data-tauri-drag-region title={t("app.name")}>
          <img src={logo} alt="" width={30} height={30} />
          <span className="rail-brand">{t("app.name")}</span>
        </div>
        {(
          [
            ["home", "home", 0],
            ["catalog", "catalog", 0],
            ["profiles", "profiles", 0],
            ["library", "library", modReport?.updates.length ?? 0],
            ["downloads", "downloads", activeDownloads.length],
            ["performance", "performance", 0],
            ["fixes", "fixes", 0],
          ] as const
        ).map(([id, icon, badge]) => (
          <button key={id} className="rail-item" data-tip={hint(id)} aria-current={view === id ? "page" : undefined} aria-label={t(`nav.${id}`)} aria-keyshortcuts={`Alt+${order.indexOf(id) + 1}`} onClick={() => setView(id)}>
            <Icon name={icon} />
            <span className="rail-label">{t(`nav.${id}`)}</span>
            {badge > 0 && <span className="rail-badge">{badge}</span>}
          </button>
        ))}
        <div className="rail-spacer" data-tauri-drag-region />
        <button className="rail-item" data-tip={hint("settings")} aria-current={view === "settings" ? "page" : undefined} aria-label={t("nav.settings")} onClick={() => setView("settings")}>
          <Icon name="settings" />
          <span className="rail-label">{t("nav.settings")}</span>
        </button>
        <button
          className="rail-item rail-fold"
          data-tip={look.rail === "open" ? t("nav.collapse") : t("nav.expand")}
          aria-label={look.rail === "open" ? t("nav.collapse") : t("nav.expand")}
          aria-expanded={look.rail === "open"}
          onClick={() => setAppearance({ rail: look.rail === "open" ? "closed" : "open" })}
        >
          <Icon name="sidebar" />
          <span className="rail-label">{t("nav.collapse")}</span>
        </button>
      </nav>

      <div className="stage">
        <header className="topbar" data-tauri-drag-region>
          <form
            className="topsearch"
            role="search"
            onSubmit={(e) => {
              e.preventDefault();
              if (view === "library") {
                setLibSearch((x) => ({ text: searchText.trim(), n: x.n + 1 }));
              } else {
                setSearch((x) => ({ text: searchText.trim(), n: x.n + 1 }));
                setView("catalog");
              }
            }}
          >
            <input
              ref={searchBox}
              className="field"
              type="search"
              value={searchText}
              placeholder={view === "library" ? t("library.search") : t("catalog.search")}
              aria-label={view === "library" ? t("library.search") : t("catalog.search")}
              aria-keyshortcuts="Control+K"
              onChange={(e) => setSearchText(e.target.value)}
            />
            <kbd className="kbd" aria-hidden="true">
              Ctrl K
            </kbd>
          </form>
          <div className="spacer" data-tauri-drag-region />
          <div className="status" data-tauri-drag-region>
            {current && (
              <button className="chip chip-btn" onClick={() => setView("downloads")} title={t("downloads.title")}>
                <Icon name="downloads" size={16} />
                <span className="stat">{`${activeDownloads.length}${currentPct != null ? ` · ${currentPct}%` : ""}`}</span>
                {currentPct != null && (
                  <span className="chip-bar" aria-hidden="true">
                    <i style={{ width: `${currentPct}%` }} />
                  </span>
                )}
              </button>
            )}
            {profiles && profiles.length > 0 && (
              <Select
                className="select-profile"
                label={t("profiles.switch")}
                value={activeProfile ? String(activeProfile.id) : ""}
                onChange={(v) => {
                  const p = profiles.find((x) => String(x.id) === v);
                  if (p && !p.active) applyProfile(p);
                }}
                options={[...(activeProfile ? [] : [{ value: "", label: t("profiles.pick") }]), ...profiles.map((p) => ({ value: String(p.id), label: p.name }))]}
              />
            )}
            {running ? (
              <span className="chip chip-live" title={t("game.runningHint")}>
                <span className="dot dot-pulse" />
                {t("game.running")}
              </span>
            ) : (
              pending && (
                <span className="chip chip-warn" title={t("game.pendingHint")}>
                  {t("game.pending")}
                </span>
              )
            )}
            <button className="btn btn-play" disabled={!game || launching || running} aria-busy={launching} onClick={() => void launchGame()} title={game ? t("game.playHint") : t("game.notFoundHint")}>
              <Icon name="play" />
              {running ? t("game.running") : launching ? t("game.starting") : t("game.play")}
            </button>
            <span className="chip" title={game === null ? t("game.notFoundHint") : game?.build_id ? t("game.build", { id: game.build_id }) : undefined}>
              <span className={`dot ${game ? "" : "off"}`} />
              {game === undefined ? "…" : game ? t("game.found") : t("game.notFound")}
              {game?.build_id && <span className="stat">{`#${game.build_id}`}</span>}
            </span>
            <History items={history} unread={unread} onOpen={() => setUnread(0)} onClear={() => setHistory([])} />
          </div>
          {isTauri && <WindowControls onMaximizedChange={setMaximized} />}
        </header>

      <main className="main">
        {update && <UpdateBanner info={update} progress={updatePct} onInstall={() => void installUpdate()} onDismiss={() => setUpdate(null)} />}
        {view === "home" ? (
          <Home
            installed={installed}
            groups={groups}
            known={known}
            report={modReport}
            busy={busy}
            onOpen={setOpen}
            onToggle={toggle}
            onToggleForeign={toggleForeign}
            onUpdate={(id) => void updateMod(id)}
            onGo={setView}
            game={game}
            launched={launched}
            onBrowse={(category, sub) => {
              setPreset((x) => ({ category, sub, n: (x?.n ?? 0) + 1 }));
              setView("catalog");
            }}
            showOnboarding={!onboardHidden}
            onHideOnboarding={() => {
              setOnboardHidden(true);
              try {
                localStorage.setItem("guddion.onboarding.hidden", "1");
              } catch {
                /* the hint just comes back next time */
              }
            }}
            onPlay={() => void launchGame()}
          />
        ) : view === "catalog" ? (
          <Catalog installedIds={installedIds} onOpen={setOpen} search={search} preset={preset} onGoSettings={() => setView("settings")} />
        ) : view === "downloads" ? (
          <Downloads
            rows={queue}
            onPause={(id) => queueCall("pause_download", id)}
            onResume={(id) => queueCall("resume_download", id)}
            onCancel={(id) => queueCall("cancel_download", id)}
            onRemove={(id) => queueCall("remove_download", id)}
            onPauseAll={() => queueCall("pause_all_downloads")}
            onResumeAll={() => queueCall("resume_all_downloads")}
            onClear={() => queueCall("clear_finished_downloads")}
            onGoCatalog={() => setView("catalog")}
          />
        ) : view === "settings" ? (
          <Settings
            game={game}
            onCheckUpdate={checkUpdate}
            onSaved={() => {
              void detectGame();
              void refresh();
            }}
          />
        ) : view === "performance" ? (
          <Performance />
        ) : view === "fixes" ? (
          <Fixes
            game={game}
            compat={compat}
            onChanged={() => {
              void refresh();
            }}
          />
        ) : view === "profiles" ? (
          <Profiles
            profiles={profiles}
            busy={busy}
            onCreate={createProfile}
            onApply={applyProfile}
            onUpdate={updateProfile}
            onRename={renameProfile}
            onDelete={deleteProfile}
            onGoLibrary={() => setView("library")}
            onExport={exportProfile}
            onImport={importProfile}
          />
        ) : (
          <Library
            mods={installed}
            groups={groups}
            known={known}
            contents={contents}
            last={last}
            busy={busy}
            onToggle={toggle}
            onToggleForeign={toggleForeign}
            onRemove={remove}
            onUndo={undo}
            onReorder={reorder}
            gameBuild={game?.build_id ?? null}
            report={modReport}
            checking={checkingMods}
            onCheckUpdates={() => void checkMods(true)}
            onUpdate={(id) => void updateMod(id)}
            onGoCatalog={() => setView("catalog")}
            compat={compat}
            gameFound={!!game}
            gameRunning={running}
            search={libSearch}
            onBulkToggle={bulkToggle}
            onBulkRemove={bulkRemove}
          />
        )}
      </main>

      </div>

      {open && <ModDrawer mod={open} installed={installedIds.has(open.id)} queued={queuedModIds.has(open.id)} busy={busy} onClose={() => setOpen(null)} onInstall={(m, f) => void install(m, f)} />}
      <Activity state={activity} onDismiss={() => setActivity(null)} />
      {confirm && <Confirm state={confirm} onCancel={() => setConfirm(null)} />}
    </div>
  );
}
