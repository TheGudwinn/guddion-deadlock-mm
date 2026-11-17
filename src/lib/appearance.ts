import { useSyncExternalStore } from "react";

/** Everything the user can change about how the launcher looks. Stored in localStorage, applied as attributes and CSS variables on <html>. */
export type Appearance = {
  /** Accent colour as a hue, 0-360. */
  hue: number;
  /** Background light: moving, still or none. */
  glow: "live" | "still" | "off";
  /** Strength of the background light, 0-100. */
  glowPower: number;
  surface: "night" | "graphite" | "black";
  motion: "full" | "calm" | "off";
  radius: "sharp" | "soft" | "round";
  density: "compact" | "normal" | "roomy";
  cards: "s" | "m" | "l";
  blur: boolean;
  /** How much the picture behind the interface is darkened, 0-90, and blurred, 0-24 px. */
  wallDim: number;
  wallBlur: number;
  /** Sidebar: labels shown or icons only. */
  rail: "open" | "closed";
  /** Blocks of the home page. */
  home: { hero: boolean; popular: boolean; mine: boolean; stats: boolean };
};

export const DEFAULTS: Appearance = {
  hue: 250,
  glow: "live",
  glowPower: 60,
  surface: "night",
  motion: "full",
  radius: "round",
  density: "normal",
  cards: "m",
  blur: true,
  wallDim: 55,
  wallBlur: 0,
  rail: "open",
  home: { hero: true, popular: true, mine: true, stats: true },
};

export const HUES = [
  { hue: 250, name: "violet" },
  { hue: 215, name: "blue" },
  { hue: 175, name: "teal" },
  { hue: 140, name: "green" },
  { hue: 38, name: "amber" },
  { hue: 12, name: "coral" },
  { hue: 335, name: "rose" },
] as const;

const KEY = "guddion.appearance";

function load(): Appearance {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return DEFAULTS;
    const p = JSON.parse(raw) as Partial<Appearance>;
    return { ...DEFAULTS, ...p, home: { ...DEFAULTS.home, ...(p.home ?? {}) } };
  } catch {
    return DEFAULTS;
  }
}

let state: Appearance = load();
const listeners = new Set<() => void>();

const RADII = { sharp: [8, 6, 4], soft: [16, 12, 10], round: [22, 16, 999] } as const;
const DENSITY = { compact: 0.78, normal: 1, roomy: 1.25 } as const;
const CARD_MIN = { s: 150, m: 200, l: 260 } as const;

/** Writes the settings to <html>. Called once at start and after every change. */
export function applyAppearance(a: Appearance = state) {
  const el = document.documentElement;
  const [card, field, pill] = RADII[a.radius];
  el.style.setProperty("--hue", String(a.hue));
  el.style.setProperty("--glow-a", String(a.glow === "off" ? 0 : a.glowPower / 100));
  el.style.setProperty("--r-card", `${card}px`);
  el.style.setProperty("--r-field", `${field}px`);
  el.style.setProperty("--r-frame", `${a.radius === "sharp" ? 14 : 28}px`);
  el.style.setProperty("--pill", `${pill}px`);
  el.style.setProperty("--d", String(DENSITY[a.density]));
  el.style.setProperty("--card-min", `${CARD_MIN[a.cards]}px`);
  el.style.setProperty("--blur", a.blur ? "blur(18px)" : "none");
  el.dataset.surface = a.surface;
  el.dataset.glow = a.glow;
  el.dataset.motion = a.motion;
  el.style.setProperty("--wall-dim", String(a.wallDim / 100));
  el.style.setProperty("--wall-blur", `${a.wallBlur}px`);
}

export function setAppearance(patch: Partial<Appearance>) {
  state = { ...state, ...patch };
  try {
    localStorage.setItem(KEY, JSON.stringify(state));
  } catch {
    /* private mode or full storage: the setting just lasts until restart */
  }
  applyAppearance(state);
  listeners.forEach((l) => l());
}

export const resetAppearance = () => {
  setWallpaper(null);
  setAppearance({ ...DEFAULTS, home: { ...DEFAULTS.home } });
};

/* ---- background picture: shrunk to a screen-sized JPEG and kept as a data URL ---- */
const WALL_KEY = "guddion.wallpaper";
let wall: string | null = null;
try {
  wall = localStorage.getItem(WALL_KEY);
} catch {
  /* storage unavailable */
}
const wallListeners = new Set<() => void>();

function setWallpaper(url: string | null) {
  wall = url;
  try {
    if (url) localStorage.setItem(WALL_KEY, url);
    else localStorage.removeItem(WALL_KEY);
  } catch {
    /* too big for storage: it stays for this session only */
  }
  wallListeners.forEach((l) => l());
}

export const clearWallpaper = () => setWallpaper(null);

/** Reads an image file, scales it down so it fits in storage, and makes it the background. */
export async function pickWallpaper(file: File): Promise<void> {
  if (!file.type.startsWith("image/")) throw new Error("not an image");
  const bmp = await createImageBitmap(file);
  const k = Math.min(1, 1920 / Math.max(bmp.width, bmp.height));
  const c = document.createElement("canvas");
  c.width = Math.round(bmp.width * k);
  c.height = Math.round(bmp.height * k);
  c.getContext("2d")!.drawImage(bmp, 0, 0, c.width, c.height);
  bmp.close();
  setWallpaper(c.toDataURL("image/jpeg", 0.82));
}

export function useWallpaper(): string | null {
  return useSyncExternalStore(
    (cb) => {
      wallListeners.add(cb);
      return () => wallListeners.delete(cb);
    },
    () => wall,
  );
}

export function useAppearance(): Appearance {
  return useSyncExternalStore(
    (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    () => state,
  );
}
