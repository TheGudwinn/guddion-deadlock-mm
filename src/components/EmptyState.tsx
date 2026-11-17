import type { ReactNode } from "react";

/** What a screen shows when there is nothing to show: what happened and what to do next. */
export default function EmptyState({ title, text, children, tone = "calm" }: { title: string; text?: string; children?: ReactNode; tone?: "calm" | "problem" }) {
  return (
    <div className={`blank blank-${tone}`} role={tone === "problem" ? "alert" : undefined}>
      <div className="blank-mark" aria-hidden="true">
        {tone === "problem" ? "!" : "·"}
      </div>
      <h2 className="blank-title">{title}</h2>
      {text && <p className="blank-text">{text}</p>}
      {children && <div className="blank-actions">{children}</div>}
    </div>
  );
}
