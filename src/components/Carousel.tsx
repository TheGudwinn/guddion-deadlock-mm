import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import Icon from "./Icons";

/** A row that scrolls sideways with arrow buttons. Arrows hide when there is nothing more in that direction. */
export default function Carousel({ title, action, children, className = "" }: { title: ReactNode; action?: ReactNode; children: ReactNode; className?: string }) {
  const { t } = useTranslation();
  const row = useRef<HTMLUListElement>(null);
  const [edge, setEdge] = useState({ start: true, end: false });

  const measure = useCallback(() => {
    const el = row.current;
    if (el) setEdge({ start: el.scrollLeft < 4, end: el.scrollLeft + el.clientWidth >= el.scrollWidth - 4 });
  }, []);
  useEffect(() => {
    measure();
    const el = row.current;
    if (!el) return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [measure, children]);

  const go = (dir: 1 | -1) => row.current?.scrollBy({ left: dir * row.current.clientWidth * 0.85, behavior: "smooth" });

  return (
    <div className={`rowbar ${className}`}>
      <div className="rowbar-head">
        <h3 className="rowbar-title">{title}</h3>
        <div className="rowbar-tools">
          <button className="btn-icon" aria-label={t("home.prev")} disabled={edge.start} onClick={() => go(-1)}>
            <Icon name="left" size={16} />
          </button>
          <button className="btn-icon" aria-label={t("home.next")} disabled={edge.end} onClick={() => go(1)}>
            <Icon name="right" size={16} />
          </button>
          {action}
        </div>
      </div>
      <ul ref={row} className="rowbar-track" onScroll={measure}>
        {children}
      </ul>
    </div>
  );
}
