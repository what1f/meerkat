import { useRef, type PointerEvent, type PointerEventHandler, type RefCallback } from 'react';
import { clamp, eatClick, follow, grabbing, threshold, useDragLife } from './pointer';
export type CarryHandle = {
  ref: RefCallback<HTMLElement>;
  onPointerDown: PointerEventHandler<HTMLElement>;
};
// The seated figure is marked `data-who`; it names its carry pose in `data-carry`.
function lift(who: HTMLElement) {
  const ghost = document.createElement('img');
  ghost.src = who.dataset.carry ?? '';
  ghost.width = Number(who.dataset.carryWidth) || 0;
  ghost.height = Number(who.dataset.carryHeight) || 0;
  ghost.alt = '';
  ghost.dataset.carryGhost = '';
  document.body.append(ghost);
  return ghost;
}
// Pick a member up by the scruff and drop them on another row.
export function useCarry(onDrop: (id: string, rowKey: string) => void): {
  seatHandle: (id: string) => CarryHandle;
  rowProps: (rowKey: string) => { ref: RefCallback<HTMLElement> };
} {
  const rows = useRef(new Map<string, HTMLElement>());
  const life = useDragLife();
  const rowOf = (el: Element | null | undefined) =>
    el ? [...rows.current].find(([, row]) => row.contains(el)) : undefined;
  const start = (id: string, e: PointerEvent<HTMLElement>) => {
    const seat = e.currentTarget;
    const who = (e.target as Element).closest<HTMLElement>('[data-who]');
    if (e.button !== 0 || !who || !seat.contains(who)) return;
    e.preventDefault();
    const home = rowOf(seat)?.[0];
    let ghost: HTMLImageElement | null = null;
    let rot = 0;
    let px = e.clientX;
    let settle: ReturnType<typeof setTimeout> | undefined;
    let over: [string, HTMLElement] | undefined;
    const mark = (next: typeof over) => {
      rows.current.forEach((row) => delete row.dataset.over);
      if (next && next[0] !== home) next[1].dataset.over = '';
      over = next;
    };
    // Float the figure back to its chair, then put it down.
    const toChair = (g: HTMLImageElement) => {
      const r = who.getBoundingClientRect();
      g.dataset.back = '';
      g.style.left = `${r.left + (r.width - g.width) / 2}px`;
      g.style.top = `${r.top}px`;
      life.later(() => {
        g.remove();
        delete seat.dataset.lifted;
      }, 280);
    };
    // Cancelled or unmounted: the figure goes back to its chair, nothing is committed.
    const restore = () => {
      const g = ghost;
      if (!g) return;
      ghost = null;
      clearTimeout(settle);
      grabbing(false);
      mark(undefined);
      toChair(g);
    };
    const track = follow(e, seat, {
      move: (ev) => {
        if (!ghost) {
          if (Math.hypot(ev.clientX - e.clientX, ev.clientY - e.clientY) < threshold) return;
          if (!track.capture()) return abort();
          grabbing(true);
          ghost = lift(who);
          seat.dataset.lifted = '';
        }
        // Swing against the direction of travel, like something dangling.
        rot = clamp(rot * 0.8 - (ev.clientX - px) * 0.9, -20, 20);
        px = ev.clientX;
        const g = ghost;
        delete g.dataset.settle;
        g.style.left = `${ev.clientX - g.width / 2}px`;
        g.style.top = `${ev.clientY - 10}px`;
        g.style.transform = `rotate(${rot}deg)`;
        clearTimeout(settle);
        settle = setTimeout(() => {
          rot = 0;
          g.dataset.settle = '';
          g.style.transform = '';
        }, 90);
        mark(rowOf(document.elementsFromPoint(ev.clientX, ev.clientY)[0]));
      },
      up: () => {
        life.end();
        const g = ghost;
        if (!g) return;
        ghost = null;
        clearTimeout(settle);
        grabbing(false);
        eatClick();
        const target = over;
        mark(undefined);
        if (target && target[0] !== home) {
          g.remove();
          delete seat.dataset.lifted;
          onDrop(id, target[0]);
          return;
        }
        // Not dropped on another row: back to the chair.
        toChair(g);
      },
      cancel: () => {
        life.end();
        restore();
      },
    });
    const abort = () => {
      life.end();
      track.stop();
      restore();
    };
    life.begin(abort);
  };
  return {
    seatHandle: (id) => ({
      ref: () => {},
      onPointerDown: (e) => start(id, e),
    }),
    rowProps: (rowKey) => ({
      ref: (el) => {
        if (!el) return;
        rows.current.set(rowKey, el);
        return () => {
          rows.current.delete(rowKey);
        };
      },
    }),
  };
}
