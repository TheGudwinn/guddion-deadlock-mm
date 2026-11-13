import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import {
  call,
  isTauri,
  type ApplyResult,
  type Profile,
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
import Activity, { type ActivityState } from "./components/Activity";
import UpdateBanner from "./components/UpdateBanner";
import Catalog from "./features/store/Catalog";
import ModDrawer from "./features/store/ModDrawer";
import Library from "./features/library/Library";
import Profiles from "./features/profiles/Profiles";
import Settings from "./features/settings/Settings";
import Downloads from "./features/downloads/Downloads";

type View = "catalog" | "library" | "profiles" | "downloads" | "settings";

export default function App() {
  const { t } = useTranslation();
  const [view, setView] = useState<View>("catalog");
  const [game, setGame] = useState<GameInstall | null | undefined>(undefined);
  const [installed, setInstalled] = useState<InstalledMod[] | null>(null);
  const [foreign, setForeign] = useState<ForeignAddon[]>([]);
  const [last, setLast] = useState<LastAction | null>(null);
  const [profiles, setProfiles] = useState<Profile[] | null>(null);
  const [open, setOpen] = useState<Mod | null>(null);
  const [busy, setBusy] = useState(false);
  const [activity, setActivity] = useState<ActivityState | null>(null);
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [queue, setQueue] = useState<QueueRow[]>([]);
  const prevStates = useRef<Map<number, string>>(new Map());
  const [modReport, setModReport] = useState<UpdateReport | null>(null);
  const [checkingMods, setCheckingMods] = useState(false);
  const autoChecked = useRef(false);
  const [updatePct, setUpdatePct] = useState<number | null>(null);

  const refresh = useCallback(async () => {
    const [mods, f, l, pr] = await Promise.all([
      call<InstalledMod[]>("list_installed").catch(() => null),
      call<ForeignAddon[]>("list_foreign_addons").catch(() => []),
      call<LastAction | null>("last_action").catch(() => null),
      call<Profile[]>("list_profiles").catch(() => null),
    ]);
    setProfiles(pr);
    setInstalled(mods);
    setForeign(f);
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
      } else if (r.state === "failed") {
        setActivity({ kind: "error", text: `${r.name}: ${humanizeError(t, r.error ?? "")}` });
      }
    }
    prevStates.current = new Map(rows.map((r) => [r.id, r.state]));
    setQueue(rows);
  }, [t, refresh]);

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

  const applyProfile = (p: Profile) =>
    void run(async () => {
      const r = await call<ApplyResult>("apply_profile", { id: p.id });
      say(t("profiles.applied", { name: p.name, on: r.enabled, off: r.disabled }), r.missing.length > 0 ? t("profiles.appliedMissing", { count: r.missing.length }) : undefined);
    });

  const updateProfile = (p: Profile) =>
    void run(async () => {
      await call("update_profile_from_current", { id: p.id });
      say(t("profiles.updated", { name: p.name }));
    });

  const deleteProfile = (id: number) => void run(() => call("delete_profile", { id }));

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

  const toggle = (id: number, enabled: boolean) => void run(() => call("set_mod_enabled", { id, enabled }));
  const remove = (id: number) => void run(() => call("uninstall_mod", { id }));
  const reorder = (ids: number[]) =>
    void run(async () => {
      await call("reorder_mods", { order: ids });
      say(t("library.reordered"));
    });
  const undo = () =>
    void run(async () => {
      await call("undo_last");
      setActivity({ kind: "ok", text: t("library.undone") });
    });

  return (
    <div className="shell">
      <header className="topbar">
        <div className="brand">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" aria-hidden="true">
            <path d="M12 2.5 21 12l-9 9.5L3 12 12 2.5Z" stroke="currentColor" strokeWidth="1.8" strokeLinejoin="round" />
            <path d="M12 8v8M8.5 12h7" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" />
          </svg>
          {t("app.name")}
        </div>
        <nav className="nav">
          <button className="nav-item" aria-current={view === "catalog" ? "page" : undefined} onClick={() => setView("catalog")}>
            {t("nav.catalog")}
          </button>
          <button className="nav-item" aria-current={view === "profiles" ? "page" : undefined} onClick={() => setView("profiles")}>
            {t("nav.profiles")}
          </button>
          <button className="nav-item" aria-current={view === "library" ? "page" : undefined} onClick={() => setView("library")}>
            {t("nav.library")}
            {installed && installed.length > 0 && <span className="nav-count">{installed.length}</span>}
            {modReport && modReport.updates.length > 0 && <span className="nav-count nav-count-up">↑{modReport.updates.length}</span>}
          </button>
          <button className="nav-item" aria-current={view === "downloads" ? "page" : undefined} onClick={() => setView("downloads")}>
            {t("nav.downloads")}
            {activeDownloads.length > 0 && <span className="nav-count">{activeDownloads.length}</span>}
          </button>
          <button className="nav-item" aria-current={view === "settings" ? "page" : undefined} onClick={() => setView("settings")}>
            {t("nav.settings")}
          </button>
        </nav>
        <div className="status">
          {profiles?.some((x) => x.active) && (
            <span className="chip">
              <b>{profiles.find((x) => x.active)?.name}</b>
            </span>
          )}
          <span className="chip" title={game === null ? t("game.notFoundHint") : undefined}>
            <span className={`dot ${game ? "" : "off"}`} />
            {game === undefined ? "…" : game ? t("game.found") : t("game.notFound")}
            {game?.build_id && <span className="stat">{t("game.build", { id: game.build_id })}</span>}
          </span>
        </div>
      </header>

      <main className="main">
        {update && <UpdateBanner info={update} progress={updatePct} onInstall={() => void installUpdate()} onDismiss={() => setUpdate(null)} />}
        {view === "catalog" ? (
          <Catalog installedIds={installedIds} onOpen={setOpen} />
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
        ) : view === "profiles" ? (
          <Profiles
            profiles={profiles}
            busy={busy}
            onCreate={createProfile}
            onApply={applyProfile}
            onUpdate={updateProfile}
            onRename={renameProfile}
            onDelete={deleteProfile}
            onExport={exportProfile}
            onImport={importProfile}
          />
        ) : (
          <Library
            mods={installed}
            foreign={foreign}
            last={last}
            busy={busy}
            onToggle={toggle}
            onRemove={remove}
            onUndo={undo}
            onReorder={reorder}
            gameBuild={game?.build_id ?? null}
            report={modReport}
            checking={checkingMods}
            onCheckUpdates={() => void checkMods(true)}
            onUpdate={(id) => void updateMod(id)}
            onGoCatalog={() => setView("catalog")}
          />
        )}
      </main>

      {open && <ModDrawer mod={open} installed={installedIds.has(open.id)} queued={queuedModIds.has(open.id)} busy={busy} onClose={() => setOpen(null)} onInstall={(m, f) => void install(m, f)} />}
      <Activity state={activity} onDismiss={() => setActivity(null)} />
    </div>
  );
}
