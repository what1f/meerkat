import { useEffect, useRef, useState, type ReactNode } from 'react';
import css from './OptionColumn.module.css';

// How close to the right edge the pointer brings the thumb up.
const edge = 24;
/** A listbox that scrolls with a thin overlay thumb instead of the native bar. */
export function Scroller({ label, children }: { label: string; children: ReactNode }) {
  const list = useRef<HTMLDivElement>(null);
  const settle = useRef<() => void>(() => {});
  const [thumb, setThumb] = useState<{ top: number; height: number } | null>(null);
  const [scrolling, setScrolling] = useState(false);
  const [near, setNear] = useState(false);
  const measure = () => {
    const el = list.current;
    if (!el || el.scrollHeight <= el.clientHeight + 1) return setThumb(null);
    const height = Math.max(24, (el.clientHeight / el.scrollHeight) * el.clientHeight);
    const room = el.clientHeight - height;
    const top = (el.scrollTop / (el.scrollHeight - el.clientHeight)) * room;
    setThumb({ top, height });
  };
  useEffect(() => {
    const el = list.current;
    if (!el) return;
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(el);
    for (const child of Array.from(el.children)) observer.observe(child);
    return () => {
      observer.disconnect();
      settle.current();
    };
  }, [children]);
  return (
    <div
      className={css.scroller}
      onPointerMove={(e) =>
        setNear(e.currentTarget.getBoundingClientRect().right - e.clientX <= edge)
      }
      onPointerLeave={() => setNear(false)}
    >
      <div
        ref={list}
        className={css.list}
        role="listbox"
        aria-label={label}
        onScroll={() => {
          measure();
          setScrolling(true);
          settle.current();
          const timer = setTimeout(() => setScrolling(false), 700);
          settle.current = () => clearTimeout(timer);
        }}
      >
        {children}
      </div>
      {thumb && (
        <span
          className={css.thumb}
          data-shown={scrolling || near}
          style={{ height: thumb.height, transform: `translateY(${thumb.top}px)` }}
        />
      )}
    </div>
  );
}
