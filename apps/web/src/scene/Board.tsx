import { useLayoutEffect, useRef, type ReactNode } from 'react';
import type { UIProps } from '../design/index';
import css from './Board.module.css';
export type BoardProps = Omit<UIProps, 'className' | 'style'> & {
  layout: 'rails' | 'free' | 'open';
  holes?: 'on' | 'fade';
  // Sits above the board's items in the flow; a free board's holes count from below it.
  lead?: ReactNode;
  children: ReactNode;
};
// Hole pitch; the first peg on the board sits exactly on a hole.
const cell = 24;
const offset = (v: number) => `${((((v - cell / 2) % cell) + cell) % cell).toFixed(2)}px`;
export function Board({ layout, holes = 'on', lead, children, ...rest }: BoardProps) {
  const ref = useRef<HTMLElement>(null);
  const itemsRef = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const root = ref.current;
    const items = itemsRef.current;
    if (!root || !items) return;
    const fit = () => {
      // An item in the hand or settling is somewhere else: it neither places the holes nor
      // resizes the board, so the wall stays put under a drag.
      const moving = root.querySelector('[data-dragging], [data-settling]');
      if (moving) return;
      const peg = root.querySelector('[data-peg]');
      if (peg) {
        const r = root.getBoundingClientRect();
        const p = peg.getBoundingClientRect();
        root.style.setProperty('--bx', offset(p.left + p.width / 2 - r.left));
        root.style.setProperty('--by', offset(p.top + p.height / 2 - r.top));
      }
      // Pegged items are positioned absolutely; the board grows to hold them.
      if (layout === 'free') {
        const bottoms = [...items.children].map((c) => {
          const el = c as HTMLElement;
          return el.offsetTop + el.offsetHeight;
        });
        items.style.minHeight = `${Math.max(0, ...bottoms) + cell * 4}px`;
      } else items.style.minHeight = '';
    };
    fit();
    const observer = new ResizeObserver(fit);
    observer.observe(root);
    [...items.children].forEach((c) => observer.observe(c));
    return () => observer.disconnect();
  });
  return (
    <section {...rest} ref={ref} className={css.root} data-layout={layout} data-holes={holes}>
      {lead}
      <div ref={itemsRef} className={css.items}>
        {children}
      </div>
    </section>
  );
}
