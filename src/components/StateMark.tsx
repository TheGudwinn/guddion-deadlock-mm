import type { GroupState } from "../lib/foreign";

/**
 * The on/off control for a mod: a check in a green disc when the game loads it, an empty ring when it is
 * switched off, a dash when only some of its files are on. Small enough for the narrow home panel.
 * With `onClick` it is a checkbox button that switches the mod; without, a plain indicator.
 */
export default function StateMark({ state, label, onClick, disabled }: { state: GroupState; label: string; onClick?: () => void; disabled?: boolean }) {
  const icon =
    state === "on" ? (
      <svg viewBox="0 0 16 16" aria-hidden="true">
        <path d="M3.6 8.4 6.7 11.5 12.4 4.8" />
      </svg>
    ) : state === "partial" ? (
      <svg viewBox="0 0 16 16" aria-hidden="true">
        <path d="M4.5 8h7" />
      </svg>
    ) : null;

  if (!onClick) {
    return (
      <span className={`mark mark-${state}`} role="img" aria-label={label} title={label}>
        {icon}
      </span>
    );
  }
  return (
    <button type="button" className={`mark mark-btn mark-${state}`} role="checkbox" aria-checked={state === "partial" ? "mixed" : state === "on"} aria-label={label} title={label} disabled={disabled} onClick={onClick}>
      {icon}
    </button>
  );
}
