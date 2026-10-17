import { useState, type RefCallback } from 'react';
import type { PegPosition } from '../Hang';
import { clamp, eatClick, follow, grabbing, swing, threshold, trail, useDragLife } from './pointer';
export type HolePosition = { x: number; y: number };
export type HoleItem = { key: string; kind: string; pos: HolePosition };
export type HoleHandle = { ref: RefCallback<HTMLElement>; style: PegPosition; dragging: boolean };
// Hole pitch on the pegboard. An item's peg sits on the hole at its position.
const cell = 24;
const pegX = (p: HolePosition) => p.x * cell + cell / 2;
type Rect = { x: number; y: number; w: number; h: number };
const hits = (a: Rect, b: Rect) =>
  a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
const rectAt = (el: HTMLElement, p: HolePosition): Rect => ({
  x: pegX(p) - el.offsetWidth / 2,
  y: p.y * cell,
  w: el.offsetWidth,
  h: el.offsetHeight,
});
// Drag a pegged item to another hole. A short press is still a click.
export function useHoleDrag(
  items: HoleItem[],
  onMove: (key: string, pos: HolePosition) => void,
): Record<string, HoleHandle> {
  const [dragging, setDragging] = useState<string | null>(null);
  const life = useDragLife();
  // Ease an item onto a hole, then let it swing.
  const settle = (el: HTMLElement, p: HolePosition) => {
    el.dataset.settling = '';
    el.style.left = `${pegX(p)}px`;
    el.style.top = `${p.y * cell}px`;
    el.style.rotate = '';
    life.later(() => {
      delete el.dataset.settling;
      setDragging(null);
      swing(el);
    }, 280);
  };
  const elements = new Map<string, HTMLElement>();
  const drop = (item: HoleItem, el: HTMLElement) => {
    const box = el.parentElement;
    const w = el.offsetWidth;
    const cols = Math.floor((box?.clientWidth ?? 0) / cell);
    let p = {
      x: clamp(
        Math.round((el.offsetLeft - cell / 2) / cell),
        Math.ceil((w / 2 - cell / 2) / cell),
        cols - Math.ceil((w / 2 + cell / 2) / cell),
      ),
      y: Math.max(0, Math.round(el.offsetTop / cell)),
    };
    const blocked = items.some((other) => {
      const o = other.key !== item.key && elements.get(other.key);
      return o && hits(rectAt(o, other.pos), rectAt(el, p));
    });
    if (blocked) p = item.pos;
    settle(el, p);
    if (p.x !== item.pos.x || p.y !== item.pos.y) onMove(item.key, p);
  };
  const down = (item: HoleItem, el: HTMLElement) => (e: PointerEvent) => {
    const target = e.target as Element;
    if (e.button !== 0 || target.closest('input, textarea, button, a, label, [contenteditable]'))
      return;
    e.preventDefault();
    const l = el.offsetLeft;
    const t = el.offsetTop;
    let on = false;
    let tilt = 0;
    let last = e.clientX;
    // Cancelled or unmounted: back to the hole it came from, nothing is committed.
    const restore = () => {
      if (!on) return;
      on = false;
      grabbing(false);
      settle(el, item.pos);
    };
    const track = follow(e, el, {
      move: (ev) => {
        const dx = ev.clientX - e.clientX;
        const dy = ev.clientY - e.clientY;
        if (!on) {
          if (Math.hypot(dx, dy) < threshold) return;
          if (!track.capture()) return abort();
          on = true;
          grabbing(true);
          delete el.dataset.swing;
          setDragging(item.key);
        }
        el.style.left = `${l + dx}px`;
        el.style.top = `${Math.max(0, t + dy)}px`;
        tilt = trail(tilt, ev.clientX - last);
        last = ev.clientX;
        el.style.rotate = `${tilt.toFixed(2)}deg`;
      },
      up: () => {
        life.end();
        if (!on) return;
        grabbing(false);
        eatClick();
        drop(item, el);
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
  return Object.fromEntries(
    items.map((item) => [
      item.key,
      {
        ref: (el: HTMLElement | null) => {
          if (!el) return;
          elements.set(item.key, el);
          const listener = down(item, el);
          el.addEventListener('pointerdown', listener);
          return () => {
            elements.delete(item.key);
            el.removeEventListener('pointerdown', listener);
          };
        },
        style: { left: pegX(item.pos), top: item.pos.y * cell },
        dragging: dragging === item.key,
      },
    ]),
  );
}
