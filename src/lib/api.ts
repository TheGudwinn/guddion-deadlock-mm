import { invoke } from "@tauri-apps/api/core";

export type GameInstall = { root: string; addons_dir: string; gameinfo: string; build_id: string | null };

export type Mod = {
  source: string;
  id: string;
  name: string;
  author: string | null;
  category: string | null;
  thumbnail: string | null;
  version: string | null;
  likes: number;
  views: number;
  updated_at: number;
  page_url: string;
};
export type Page = { items: Mod[]; total: number; page: number; has_more: boolean; cached_at: number | null };
export type Category = { id: number; name: string; item_count: number; has_children: boolean };
export type Sort = "default" | "newest" | "latest_modified" | "most_liked" | "most_viewed";

export type ModFile = { id: string; name: string; size: number; url: string; md5: string | null; av_clean: boolean };
export type ModDetails = {
  summary: Mod;
  description_html: string | null;
  downloads: number | null;
  images: string[];
  files: ModFile[];
  cached_at: number | null;
};

export type Overlap = { path: string; with_addon: string; identical: boolean; other_disabled: boolean };
export type InstallResult = { mod_id: number; name: string; slot: number; overlaps: Overlap[] };
export type InstalledFile = { name: string; size: number; present: boolean; enabled: boolean };
export type InstalledMod = {
  id: number;
  source: string | null;
  name: string;
  version: string | null;
  remote_id: string | null;
  slot: number | null;
  enabled: boolean;
  game_build: string | null;
  files: InstalledFile[];
};
export type ModUpdate = { mod_id: number; name: string; installed_version: string | null; latest_version: string | null; file_name: string };
export type UpdateReport = { updates: ModUpdate[]; checked: number; failed: number; offline: boolean };
export type ProfileMod = {
  source: string;
  remote_id: string;
  name: string;
  version: string | null;
  enabled: boolean;
  position: number;
  installed: boolean;
};
export type Profile = { id: number; name: string; active: boolean; mods: ProfileMod[] };
export type ApplyResult = { enabled: number; disabled: number; missing: Omit<ProfileMod, "installed">[] };

export type ForeignAddon = { name: string; size: number; entries: number | null; error: string | null };
export type LastAction = { kind: string; mod_name: string | null };
export type ProgressEvent = { stage: "download" | "install"; downloaded: number; total: number | null };

export type LibraryStats = { foreign: number; disk_bytes: number; conflicts: number };
export type QueueState = "queued" | "downloading" | "paused" | "installing" | "done" | "failed" | "cancelled";
export type QueueRow = {
  id: number;
  remote_id: string;
  name: string;
  file_name: string;
  state: QueueState;
  downloaded: number;
  total: number | null;
  error: string | null;
  slot: number | null;
  overlaps: number | null;
};

export type ProxyMode = "system" | "none" | "custom";
export type SettingsView = {
  proxy: { mode: ProxyMode; kind: "http" | "https" | "socks5"; host: string; port: number; username: string | null; has_password: boolean };
  mirrors: string[];
  game_path: string | null;
  check_updates: boolean;
  update_endpoints: string[];
};
export type UpdateInfo = { version: string; current: string; notes: string | null };
export type DiagStep = { name: "url" | "dns" | "tcp" | "http"; state: "ok" | "failed" | "skipped"; ms: number | null; detail: string | null };
export type Verdict = "ok" | "dns_failed" | "tcp_failed" | "tls_failed" | "proxy_failed" | "http_failed" | "bad_url";
export type HostReport = { url: string; host: string; steps: DiagStep[]; verdict: Verdict };

const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** Calls a Tauri command. In a plain browser (design preview) it falls back to sample data. */
export async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (inTauri) return invoke<T>(cmd, args);
  if (import.meta.env.DEV) {
    const { mockInvoke } = await import("./mock");
    return mockInvoke<T>(cmd, args);
  }
  throw new Error("Tauri runtime is not available");
}

export const isTauri = inTauri;
