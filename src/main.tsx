import React from "react";
import ReactDOM from "react-dom/client";
import "./i18n";
import "./styles.css";
import App from "./App";
import { isTauri } from "./lib/api";
import { applyAppearance } from "./lib/appearance";

applyAppearance();

// In the native app the window is transparent and the rounded shell is the window.
if (isTauri) {
  document.documentElement.classList.add("native");
  // The browser context menu (reload, inspect, ...) has no place in an app window.
  window.addEventListener("contextmenu", (e) => e.preventDefault());
}

// Pause the moving background light while the window is not in use.
const idle = () => {
  document.documentElement.dataset.idle = document.hidden || !document.hasFocus() ? "1" : "0";
};
window.addEventListener("blur", idle);
window.addEventListener("focus", idle);
document.addEventListener("visibilitychange", idle);

// Cards lean toward the pointer and a soft light follows it. Skipped when motion is off or reduced.
let leaning: HTMLElement | null = null;
let frame = 0;
const calm = () => document.documentElement.dataset.motion === "off" || window.matchMedia("(prefers-reduced-motion: reduce)").matches;
const unlean = (el: HTMLElement | null) => {
  el?.style.removeProperty("--rx");
  el?.style.removeProperty("--ry");
};
window.addEventListener(
  "pointermove",
  (e) => {
    if (e.pointerType !== "mouse" || calm()) return;
    const target = (e.target as HTMLElement | null)?.closest<HTMLElement>(".tile, .card");
    if (leaning && leaning !== target) unlean(leaning);
    leaning = target ?? null;
    if (!target) return;
    cancelAnimationFrame(frame);
    frame = requestAnimationFrame(() => {
      const r = target.getBoundingClientRect();
      const x = (e.clientX - r.left) / r.width;
      const y = (e.clientY - r.top) / r.height;
      target.style.setProperty("--mx", `${(x * 100).toFixed(1)}%`);
      target.style.setProperty("--my", `${(y * 100).toFixed(1)}%`);
      target.style.setProperty("--ry", `${((x - 0.5) * 9).toFixed(2)}deg`);
      target.style.setProperty("--rx", `${((0.5 - y) * 9).toFixed(2)}deg`);
    });
  },
  { passive: true },
);
document.addEventListener("pointerleave", () => unlean(leaning), true);

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
