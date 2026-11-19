import { useEffect, useState } from "react";

/** Keeps an element mounted for `ms` after `open` turns false, so it can play its closing animation first. */
export function useExit(open: boolean, ms = 180) {
  const [mounted, setMounted] = useState(open);
  useEffect(() => {
    if (open) {
      setMounted(true);
      return;
    }
    const id = setTimeout(() => setMounted(false), ms);
    return () => clearTimeout(id);
  }, [open, ms]);
  return { mounted: open || mounted, closing: !open && mounted };
}
