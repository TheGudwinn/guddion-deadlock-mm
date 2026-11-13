import { useTranslation } from "react-i18next";

export type ActivityState =
  | { kind: "progress"; text: string; pct: number | null }
  | { kind: "ok"; text: string; sub?: string }
  | { kind: "error"; text: string };

export default function Activity({ state, onDismiss }: { state: ActivityState | null; onDismiss: () => void }) {
  const { t } = useTranslation();
  if (!state) return null;
  const cls = state.kind === "error" ? "activity activity-err" : state.kind === "ok" ? "activity activity-ok" : "activity";
  return (
    <div className={cls} role={state.kind === "error" ? "alert" : "status"}>
      <div className="activity-row">
        <div>
          <div className="activity-text">{state.text}</div>
          {state.kind === "ok" && state.sub && <div className="activity-sub">{state.sub}</div>}
        </div>
        {state.kind !== "progress" && (
          <button className="btn btn-quiet" onClick={onDismiss}>
            {t("activity.dismiss")}
          </button>
        )}
      </div>
      {state.kind === "progress" && (
        <div className={`bar ${state.pct == null ? "indeterminate" : ""}`}>
          <i style={state.pct == null ? undefined : { width: `${state.pct}%` }} />
        </div>
      )}
    </div>
  );
}
