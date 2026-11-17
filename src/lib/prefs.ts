import { useCallback, useEffect, useState } from "react";
import { call, type GamePrefs } from "./api";

/** Launch options and performance settings, saved on the app side (`game_prefs.json`). */
export function useGamePrefs() {
  const [prefs, setPrefs] = useState<GamePrefs | null>(null);
  const [saved, setSaved] = useState<GamePrefs | null>(null);

  useEffect(() => {
    call<GamePrefs>("get_game_prefs").then(
      (p) => {
        setPrefs(p);
        setSaved(p);
      },
      () => {},
    );
  }, []);

  const save = useCallback(async (p: GamePrefs) => {
    const r = await call<GamePrefs>("save_game_prefs", { prefs: p });
    setPrefs(r);
    setSaved(r);
    return r;
  }, []);

  const dirty = !!prefs && !!saved && JSON.stringify(prefs) !== JSON.stringify(saved);
  return { prefs, setPrefs, save, dirty };
}
