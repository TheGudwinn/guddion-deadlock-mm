import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { Profile } from "../../lib/api";
import EmptyState from "../../components/EmptyState";

type Props = {
  profiles: Profile[] | null;
  busy: boolean;
  onCreate: (name: string) => Promise<boolean>;
  onApply: (p: Profile) => void;
  onUpdate: (p: Profile) => void;
  onRename: (id: number, name: string) => Promise<boolean>;
  onDelete: (id: number) => void;
  onExport: (p: Profile) => void;
  onImport: () => void;
  onCopyCode: (p: Profile) => void;
  onImportCode: (code: string) => Promise<boolean>;
  onGoLibrary: () => void;
};

export default function Profiles({ profiles, busy, onCreate, onApply, onUpdate, onRename, onDelete, onExport, onImport, onCopyCode, onImportCode, onGoLibrary }: Props) {
  const { t } = useTranslation();
  const [name, setName] = useState("");
  const [code, setCode] = useState("");
  const [editing, setEditing] = useState<{ id: number; value: string } | null>(null);

  return (
    <section className="view">
      <header className="view-head">
        <div className="view-title-row">
          <h1 className="view-title">{t("profiles.title")}</h1>
          <button className="btn" disabled={busy} onClick={onImport}>
            {t("profiles.import")}
          </button>
        </div>
        <p className="view-sub">{t("profiles.subtitle")}</p>
        <form
          className="toolbar"
          onSubmit={async (e) => {
            e.preventDefault();
            if (await onCreate(name)) setName("");
          }}
        >
          <input className="field search" value={name} maxLength={100} placeholder={t("profiles.newName")} aria-label={t("profiles.newName")} onChange={(e) => setName(e.target.value)} />
          <button className="btn btn-primary" type="submit" disabled={busy || !name.trim()}>
            {t("profiles.create")}
          </button>
        </form>
        <form
          className="toolbar"
          onSubmit={async (e) => {
            e.preventDefault();
            if (await onImportCode(code)) setCode("");
          }}
        >
          <input className="field search" value={code} placeholder={t("profiles.codePlaceholder")} aria-label={t("profiles.codePlaceholder")} onChange={(e) => setCode(e.target.value)} />
          <button className="btn" type="submit" disabled={busy || !code.trim()}>
            {t("profiles.codeImport")}
          </button>
        </form>
      </header>

      {profiles && profiles.length === 0 && (
        <EmptyState title={t("profiles.emptyTitle")} text={t("profiles.empty")}>
          <button className="btn btn-primary" onClick={onGoLibrary}>
            {t("profiles.toLibrary")}
          </button>
          <button className="btn" disabled={busy} onClick={onImport}>
            {t("profiles.import")}
          </button>
        </EmptyState>
      )}

      <ul className="profiles">
        {(profiles ?? []).map((p) => {
          const on = p.mods.filter((m) => m.enabled).length;
          const missing = p.mods.filter((m) => !m.installed).length;
          return (
            <li key={p.id} className={`profile ${p.active ? "profile-active" : ""}`}>
              <div className="profile-main">
                {editing?.id === p.id ? (
                  <form
                    className="rename"
                    onSubmit={async (e) => {
                      e.preventDefault();
                      if (await onRename(p.id, editing.value)) setEditing(null);
                    }}
                  >
                    <input className="field" autoFocus value={editing.value} maxLength={100} aria-label={t("profiles.rename")} onChange={(e) => setEditing({ id: p.id, value: e.target.value })} />
                    <button className="btn btn-primary" type="submit" disabled={busy || !editing.value.trim()}>
                      {t("profiles.save")}
                    </button>
                    <button className="btn btn-quiet" type="button" onClick={() => setEditing(null)}>
                      {t("profiles.cancel")}
                    </button>
                  </form>
                ) : (
                  <div className="profile-name">
                    {p.name}
                    {p.active && <span className="tag-active">{t("profiles.active")}</span>}
                  </div>
                )}
                <div className="slot-meta">
                  {t("profiles.counts", { total: p.mods.length, on })}
                  {missing > 0 && <span className="missing"> · {t("profiles.missingCount", { count: missing })}</span>}
                </div>
                <details className="profile-mods">
                  <summary>{t("profiles.show")}</summary>
                  {p.mods.length === 0 && <p className="note">{t("profiles.empty_set")}</p>}
                  <ul>
                    {p.mods.map((m) => (
                      <li key={`${m.source}:${m.remote_id}`} className={m.installed ? "" : "is-missing"}>
                        <span className={`state ${m.enabled ? "state-on" : ""}`}>{m.enabled ? t("profiles.on") : t("profiles.off")}</span>
                        <span className="pm-name">{m.name}</span>
                        {!m.installed && <span className="pm-missing">{t("profiles.notInstalled")}</span>}
                      </li>
                    ))}
                  </ul>
                </details>
              </div>

              <div className="profile-actions">
                <button className="btn btn-primary" disabled={busy} onClick={() => onApply(p)}>
                  {t("profiles.apply")}
                </button>
                <button className="btn" disabled={busy} onClick={() => onUpdate(p)}>
                  {t("profiles.update")}
                </button>
                <button className="btn btn-quiet" disabled={busy} onClick={() => onCopyCode(p)}>
                  {t("profiles.copyCode")}
                </button>
                <button className="btn btn-quiet" disabled={busy} onClick={() => onExport(p)}>
                  {t("profiles.export")}
                </button>
                <button className="btn btn-quiet" disabled={busy} onClick={() => setEditing({ id: p.id, value: p.name })}>
                  {t("profiles.rename")}
                </button>
                <button className="btn btn-quiet" disabled={busy} onClick={() => onDelete(p.id)}>
                  {t("profiles.remove")}
                </button>
              </div>
            </li>
          );
        })}
      </ul>
    </section>
  );
}
