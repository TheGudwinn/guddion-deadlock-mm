import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

type Tip = { text: string; x: number; y: number; below: boolean };

const DELAY = 380;
const SEL = "[title], [data-tip]";

/**
 * One tooltip for the whole app, in the look of the theme. The browser's own title bubble is plain white, so the
 * `title` of an element is moved to `data-tip` the first time the pointer or the keyboard reaches it, and this
 * layer shows it instead. Icon buttons keep their name for screen readers through `aria-label`.
 */
export default function TooltipLayer() {
  const [tip, setTip] = useState<Tip | null>(null);
  const box = useRef<HTMLDivElement>(null);
  const timer = useRef(0);
  const current = useRef<Element | null>(null);

  useEffect(() => {
    const hide = () => {
      window.clearTimeout(timer.current);
      current.current = null;
      setTip(null);
    };
    const take = (el: Element) => {
      const title = el.getAttribute("title");
      if (title !== null) {
        if (title.trim() && !el.hasAttribute("aria-label") && !(el.textContent ?? "").trim()) el.setAttribute("aria-label", title);
        if (title.trim()) el.setAttribute("data-tip", title);
        el.removeAttribute("title");
      }
    };
    const show = (el: Element) => {
      take(el);
      const text = el.getAttribute("data-tip");
      if (!text) return;
      window.clearTimeout(timer.current);
      current.current = el;
      timer.current = window.setTimeout(() => {
        if (current.current !== el || !el.isConnected) return;
        const r = el.getBoundingClientRect();
        // above the element, or below it near the top edge; the box is measured and placed after it renders
        const below = r.top < 56;
        setTip({ text, x: r.left + r.width / 2, y: below ? r.bottom : r.top, below });
      }, DELAY);
    };
    const over = (e: PointerEvent) => {
      const el = (e.target as Element | null)?.closest?.(SEL);
      if (!el) return hide();
      if (el !== current.current) {
        setTip(null);
        show(el);
      }
    };
    const focus = (e: FocusEvent) => {
      const el = (e.target as Element | null)?.closest?.(SEL);
      if (el && (e.target as HTMLElement).matches?.(":focus-visible")) show(el);
    };
    document.addEventListener("pointerover", over);
    document.addEventListener("pointerdown", hide, true);
    document.addEventListener("focusin", focus);
    document.addEventListener("focusout", hide);
    document.addEventListener("scroll", hide, true);
    document.addEventListener("keydown", hide);
    window.addEventListener("blur", hide);
    return () => {
      hide();
      document.removeEventListener("pointerover", over);
      document.removeEventListener("pointerdown", hide, true);
      document.removeEventListener("focusin", focus);
      document.removeEventListener("focusout", hide);
      document.removeEventListener("scroll", hide, true);
      document.removeEventListener("keydown", hide);
      window.removeEventListener("blur", hide);
    };
  }, []);

  // keeps the bubble inside the window once its size is known
  useEffect(() => {
    const b = box.current;
    if (!tip || !b) return;
    const w = b.offsetWidth;
    const h = b.offsetHeight;
    const left = Math.max(8, Math.min(tip.x - w / 2, window.innerWidth - w - 8));
    const top = tip.below ? tip.y + 10 : tip.y - h - 10;
    b.style.left = left + "px";
    b.style.top = Math.max(8, top) + "px";
    b.style.visibility = "visible";
  }, [tip]);

  if (!tip) return null;
  return createPortal(
    <div ref={box} className="tip" role="tooltip" style={{ left: 0, top: 0, visibility: "hidden" }}>
      {tip.text}
    </div>,
    document.body,
  );
}
