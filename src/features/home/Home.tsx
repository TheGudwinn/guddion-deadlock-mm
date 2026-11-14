import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { call, type InstalledMod, type LibraryStats, type Mod, type Page, type UpdateReport } from "../../lib/api";
import { compact, formatBytes } from "../../lib/format";
import Icon from "../../components/Icons";

type Props = {
  installed: InstalledMod[] | null;
  report: UpdateReport | null;
  stats: LibraryStats | null;
  busy: boolean;
  onOpen: (m: Mod) => void;
  onToggle: (id: number, enabled: boolean) => void;
  onUpdate: (id: number) => void;
  onGo: (view: "catalog" | "library") => void;
};

/** GameBanana serves a 530 px thumbnail by default; an 800 px version of the same file exists and is sharp enough for the hero. */
const larger = (url: string) => url.replace("/530-90_", "/800-90_");

function Ring({ value, label, caption }: { value: number | null; label: string; caption: string }) {
  const pct = value == null ? 0 : Math.round(value * 100);
  return (
    <div className="ring-wrap">
      <div className="ring" style={{ ["--p" as string]: `${pct}%` }} role="img" aria-label={`${caption}: ${value == null ? "-" : pct + "%"}`}>
        <span>{value == null ? "–" : `${pct}%`}</span>
      </div>
      <div className="ring-caption">{label}</div>
    </div>
  );
}

export default function Home({ installed, report, stats, busy, onOpen, onToggle, onUpdate, onGo }: Props) {
  const { t } = useTranslation();
  const [popular, setPopular] = useState<Mod[] | null>(null);
  const [offline, setOffline] = useState(false);

  useEffect(() => {
    let stale = false;
    call<Page>("search_mods", { query: { text: null, category: null, sort: "most_liked", page: 1, per_page: 9 } }).then(
      (p) => {
        if (stale) return;
        setPopular(p.items);
        setOffline(p.cached_at != null);
      },
      () => !stale && setPopular([]),
    );
    return () => {
      stale = true;
    };
  }, []);

  const hero = popular?.[0];
  const rest = (popular ?? []).slice(1);
  const mods = installed ?? [];
  const enabled = mods.filter((m) => m.enabled).length;
  const updateOf = (id: number) => report?.updates.find((u) => u.mod_id === id);
  const upToDate = report && report.checked > 0 ? (report.checked - report.updates.length) / report.checked : null;
  const disk = mods.reduce((s, m) => s + m.files.reduce((a, f) => a + f.size, 0), 0);

  return (
    <section className="view">
      <header className="view-head">
        <h1 className="view-title">{t("home.title")}</h1>
        <p className="view-sub">{t("home.subtitle")}</p>
      </header>

      <div className="home">
        <div className="hero" style={hero?.thumbnail ? { ["--cover" as string]: `url("${larger(hero.thumbnail)}")` } : undefined}>
          <div className="hero-body">
            <span className="tag-pill">{t("home.popularNow")}</span>
            {hero ? (
              <>
                <h2 className="hero-title">{hero.name}</h2>
                <p className="hero-meta">
                  {[hero.author, hero.category].filter(Boolean).join(" · ")}
                  <span className="like like-inline">
                    <Icon name="heart" /> {compact(hero.likes)}
                  </span>
                </p>
                <div className="row">
                  <button className="btn btn-primary" onClick={() => onOpen(hero)}>
                    {t("home.open")}
                  </button>
                  <button className="btn" onClick={() => onGo("catalog")}>
                    {t("home.toCatalog")}
                  </button>
                </div>
              </>
            ) : (
              <>
                <h2 className="hero-title">{popular ? t("home.noCatalog") : "…"}</h2>
                {popular && popular.length === 0 && <p className="hero-meta">{t("home.noCatalogHint")}</p>}
              </>
            )}
            {offline && <p className="note note-warn">{t("home.offline")}</p>}
          </div>
        </div>

        <div className="popular">
          <h3 className="panel-title">{t("home.popular")}</h3>
          <ul className="grid">
            {rest.map((m) => (
              <li key={m.id}>
                <button className="card" onClick={() => onOpen(m)}>
                  {m.thumbnail ? (
                    <img src={m.thumbnail} alt="" loading="lazy" />
                  ) : (
                    <span className="card-ph" aria-hidden="true">
                      {m.name.slice(0, 1)}
                    </span>
                  )}
                  <span className="card-shade" />
                  <span className="like">
                    <Icon name="heart" /> {compact(m.likes)}
                  </span>
                  <span className="card-body">
                    <span className="card-title">{m.name}</span>
                    <span className="card-meta">{[m.author, m.category].filter(Boolean).join(" · ")}</span>
                  </span>
                  <span className="go" aria-hidden="true">
                    ↗
                  </span>
                </button>
              </li>
            ))}
          </ul>
          {popular && rest.length === 0 && <p className="note">{t("home.noCatalog")}</p>}
        </div>

        <aside className="side">
          <div className="panel">
            <h3 className="panel-title">{t("home.mine")}</h3>
            {mods.length === 0 ? (
              <div className="empty-panel">
                <p>{t("library.empty")}</p>
                <button className="btn btn-primary" onClick={() => onGo("catalog")}>
                  {t("library.toCatalog")}
                </button>
              </div>
            ) : (
              <ul className="mine-list">
                {mods.slice(0, 6).map((m) => {
                  const up = updateOf(m.id);
                  return (
                    <li key={m.id} className={`mine-row ${m.enabled ? "" : "mine-off"}`}>
                      <div className="plate plate-sm">
                        <b>{m.slot != null ? String(m.slot).padStart(2, "0") : "??"}</b>
                      </div>
                      <div className="mine-name">
                        <b>{m.name}</b>
                        <small>{[m.version && `v${m.version}`, m.enabled ? t("library.enabled") : t("library.disabled")].filter(Boolean).join(" · ")}</small>
                        {up && (
                          <button className="btn btn-sm btn-primary mine-update" disabled={busy} title={t("library.updateAvailable", { version: up.latest_version ? ` ${up.latest_version}` : "" })} onClick={() => onUpdate(m.id)}>
                            {t("library.update")}
                            {up.latest_version ? ` → ${up.latest_version}` : ""}
                          </button>
                        )}
                      </div>
                      <button className="switch" role="switch" aria-checked={m.enabled} aria-label={`${t("library.toggle")}: ${m.name}`} title={m.enabled ? t("home.disable") : t("home.enable")} disabled={busy} onClick={() => onToggle(m.id, !m.enabled)} />
                    </li>
                  );
                })}
              </ul>
            )}
            {mods.length > 6 && (
              <button className="btn btn-quiet" onClick={() => onGo("library")}>
                {t("home.allMods", { count: mods.length })}
              </button>
            )}
          </div>

          <div className="panel">
            <h3 className="panel-title">{t("home.stats")}</h3>
            <div className="rings">
              <Ring value={mods.length ? enabled / mods.length : null} label={t("home.enabled", { on: enabled, total: mods.length })} caption={t("home.ringEnabled")} />
              <Ring value={upToDate} label={report ? t("home.uptodate", { count: report.updates.length }) : t("home.notChecked")} caption={t("home.ringUpToDate")} />
            </div>
            <dl className="facts">
              <div>
                <dt>{t("home.disk")}</dt>
                <dd>{formatBytes(stats?.disk_bytes ?? disk)}</dd>
              </div>
              <div className={stats && stats.conflicts > 0 ? "fact-warn" : ""}>
                <dt>{t("home.conflicts")}</dt>
                <dd>{stats ? stats.conflicts : "–"}</dd>
              </div>
              <div>
                <dt>{t("home.foreign")}</dt>
                <dd>{stats ? stats.foreign : "–"}</dd>
              </div>
            </dl>
          </div>
        </aside>
      </div>
    </section>
  );
}
