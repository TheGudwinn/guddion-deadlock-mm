import { isTauri } from "./api";

/** Opens a web page in the system browser (a new tab when running in the plain browser preview). */
export async function openExternal(url: string) {
  if (isTauri) {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(url);
  } else {
    window.open(url, "_blank", "noopener");
  }
}
