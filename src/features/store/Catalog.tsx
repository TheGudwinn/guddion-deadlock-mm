import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { call, type Category, type Mod, type Page, type Sort } from "../../lib/api";
import { compact, humanizeError } from "../../lib/format";

type Props = {
  installedIds: Set<string>;
  onOpen: (m: Mod) => void;
  /** Text typed in the top-bar search; `n` changes on every submit so the same text can be sent again. */
  search?: { text: string; n: number };
};

const SORTS: Exclude<Sort, "default">[] = ["most_liked", "newest", "latest_modified", "most_viewed"];

function Heart() {
  return (
    <svg width="12" height="12" viewBox="0 0 16 16" aria-hidden="true" fill="currentColor">
      <path d="M8 14.2 2.3 8.6a3.6 3.6 0 0 1 5.1-5.1L8 4.1l.6-.6a3.6 3.6 0 0 1 5.1 5.1L8 14.2Z" />
    </svg>
  );
}

export default function Catalog({ installedIds, onOpen, search }: Props) {
  const { t } = useTranslation();
  const [text, setText] = useState(search?.text ?? "");
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState<number | null>(null);
  const [sort, setSort] = useState<Sort>("most_liked");
  const [page, setPage] = useState(1);
  const [cats, setCats] = useState<Category[]>([]);
  const [data, setData] = useState<Page | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [reload, setReload] = useState(0);

  useEffect(() => {
    call<Category[]>("list_categories", { parent: null }).then(setCats, () => {});
  }, []);

  useEffect(() => {
    if (search) setText(search.text);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [search?.n]);

  // Debounce typing so every keystroke is not a request.
  useEffect(() => {
    const id = setTimeout(() => {
      setQuery(text.trim());
      setPage(1);
    }, 400);
    return () => clearTimeout(id);
  }, [text]);

  useEffect(() => {
    let stale = false;
    setLoading(true);
    setError(null);
    call<Page>("search_mods", { query: { text: query || null, category, sort, page, per_page: 24 } }).then(
      (res) => {
        if (stale) return;
        setData(res);
        setLoading(false);
      },
      (e) => {
        if (stale) return;
        setError(humanizeError(t, e));
        setLoading(false);
      },
    );
    return () => {
      stale = true;
    };
  }, [query, category, sort, page, reload, t]);

  const searching = query.length > 0;

  return (
    <section className="view">
      <header className="view-head">
        <div className="view-title-row">
          <h1 className="view-title">{t("catalog.title")}</h1>
          {data && <span className="stat">{t("catalog.found", { count: data.total })}</span>}
        </div>
        <div className="toolbar">
          <input className="field search" type="search" value={text} placeholder={t("catalog.search")} aria-label={t("catalog.search")} onChange={(e) => setText(e.target.value)} />
          <select
            className="field"
            value={sort}
            disabled={searching}
            aria-label={t("catalog.sort.most_liked")}
            onChange={(e) => {
              setSort(e.target.value as Sort);
              setPage(1);
            }}
          >
            {SORTS.map((s) => (
              <option key={s} value={s}>
                {t(`catalog.sort.${s}`)}
              </option>
            ))}
          </select>
        </div>
        <div className="cats" role="group" aria-label={t("catalog.allCategories")}>
          {[{ id: null as number | null, name: t("catalog.allCategories") }, ...cats].map((c) => (
            <button
              key={c.id ?? "all"}
              className="cat"
              aria-pressed={category === c.id}
              disabled={searching}
              onClick={() => {
                setCategory(c.id);
                setPage(1);
              }}
            >
              {c.name}
            </button>
          ))}
        </div>
        {searching && <p className="note">{t("catalog.searchNote")}</p>}
        {data?.cached_at && <p className="note note-warn">{t("catalog.offline", { date: new Date(data.cached_at * 1000).toLocaleString() })}</p>}
      </header>

      {error && (
        <div className="banner" role="alert">
          <span>{error}</span>
          <button className="btn" onClick={() => setReload((n) => n + 1)}>
            {t("catalog.retry")}
          </button>
        </div>
      )}

      {loading && !data && (
        <ul className="grid" aria-hidden="true">
          {Array.from({ length: 8 }, (_, i) => (
            <li key={i}>
              <div className="skeleton" />
            </li>
          ))}
        </ul>
      )}

      {data && data.items.length === 0 && !loading && <p className="empty">{t("catalog.empty")}</p>}

      {data && data.items.length > 0 && (
        <>
          <ul className="grid" style={{ opacity: loading ? 0.55 : 1 }}>
            {data.items.map((m, i) => (
              <li key={m.id} className={i === 0 && !searching && page === 1 ? "tile-feature" : undefined}>
                <button className="card" onClick={() => onOpen(m)}>
                  {m.thumbnail ? (
                    <img src={m.thumbnail} alt="" loading="lazy" />
                  ) : (
                    <span className="card-ph" aria-hidden="true">
                      {m.name.slice(0, 1)}
                    </span>
                  )}
                  <span className="card-shade" />
                  {installedIds.has(m.id) && <span className="badge">{t("catalog.installed")}</span>}
                  <span className="like">
                    <Heart /> {compact(m.likes)}
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
          <div className="pager">
            <button className="btn" disabled={page <= 1 || loading} onClick={() => setPage(page - 1)}>
              {t("catalog.prev")}
            </button>
            <span className="stat">{t("catalog.page", { n: page })}</span>
            <button className="btn" disabled={!data.has_more || loading} onClick={() => setPage(page + 1)}>
              {t("catalog.next")}
            </button>
          </div>
        </>
      )}
    </section>
  );
}
