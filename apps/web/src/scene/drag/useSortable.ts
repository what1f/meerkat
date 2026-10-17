import {
  useLayoutEffect,
  useRef,
  type PointerEvent,
  type PointerEventHandler,
  type RefCallback,
} from 'react';
import { eatClick, follow, grabbing, swing, threshold, trail, useDragLife } from './pointer';
export type SortableHandle = {
  ref: RefCallback<HTMLElement>;
  onPointerDown: PointerEventHandler<HTMLElement>;
};
const room = { duration: 320, easing: 'cubic-bezier(.22, 1, .36, 1)' };
// Items stay in the grid. Dragging one only changes the order; the others make room.
// While dragging, the order lives in CSS `order` so React keeps owning the DOM.
export function useSortable(
  ids: string[],
  onReorder: (ids: string[]) => void,
): { itemProps: (id: string) => SortableHandle } {
  const elements = useRef(new Map<string, HTMLElement>());
  const active = useRef(false);
  const life = useDragLife();
  // Let the dragged item ease back into its cell; the order styles go once it lands.
  const land = (el: HTMLElement) => {
    el.dataset.settling = '';
    el.style.translate = '';
    el.style.rotate = '';
    life.later(() => {
      delete el.dataset.settling;
      delete el.dataset.dragging;
      active.current = false;
      elements.current.forEach((o) => (o.style.order = ''));
      swing(el);
    }, 300);
  };
  useLayoutEffect(() => {
    if (!active.current) elements.current.forEach((el) => (el.style.order = ''));
  }, [ids]);
  const start = (id: string, e: PointerEvent<HTMLElement>) => {
    const el = e.currentTarget;
    const grid = el.parentElement;
    if (
      e.button !== 0 ||
      !grid ||
      (e.target as Element).closest('input, textarea, [contenteditable]')
    )
      return;
    e.preventDefault();
    const order = [...ids];
    const els = () => order.map((key) => elements.current.get(key)).filter((x) => !!x);
    // Negative orders keep trailing non-sortable siblings, like an add slot, at the end.
    const place = () => els().forEach((o, i) => (o.style.order = String(i - order.length)));
    let on = false;
    let tilt = 0;
    let last = e.clientX;
    let gx = 0;
    let gy = 0;
    // Cancelled or unmounted: the original order comes back, nothing is committed.
    const restore = () => {
      if (!on) return;
      on = false;
      grabbing(false);
      order.splice(0, order.length, ...ids);
      place();
      land(el);
    };
    const track = follow(e, el, {
      move: (ev) => {
        if (!on) {
          if (Math.hypot(ev.clientX - e.clientX, ev.clientY - e.clientY) < threshold) return;
          if (!track.capture()) return abort();
          on = true;
          active.current = true;
          grabbing(true);
          delete el.dataset.swing;
          el.dataset.dragging = '';
          const g = grid.getBoundingClientRect();
          gx = e.clientX - g.left - el.offsetLeft;
          gy = e.clientY - g.top - el.offsetTop;
        }
        const g = grid.getBoundingClientRect();
        const x = ev.clientX - g.left - gx;
        const y = ev.clientY - g.top - gy;
        const sibs = els();
        const d2 = (o: HTMLElement) => (o.offsetLeft - x) ** 2 + (o.offsetTop - y) ** 2;
        const target = sibs.reduce((a, o) => (d2(o) < d2(a) ? o : a), el);
        if (target !== el) {
          const before = new Map(sibs.map((o) => [o, [o.offsetLeft, o.offsetTop]]));
          const from = sibs.indexOf(el);
          const to = sibs.indexOf(target);
          order.splice(to, 0, ...order.splice(from, 1));
          place();
          sibs.forEach((o) => {
            const [l = 0, t = 0] = before.get(o) ?? [];
            const dx = l - o.offsetLeft;
            const dy = t - o.offsetTop;
            if (o !== el && (dx || dy))
              o.animate(
                [{ transform: `translate(${dx}px, ${dy}px)` }, { transform: 'none' }],
                room,
              );
          });
        }
        el.style.translate = `${x - el.offsetLeft}px ${y - el.offsetTop}px`;
        tilt = trail(tilt, ev.clientX - last);
        last = ev.clientX;
        el.style.rotate = `${tilt.toFixed(2)}deg`;
      },
      up: () => {
        life.end();
        if (!on) return;
        grabbing(false);
        eatClick();
        land(el);
        if (order.some((key, i) => key !== ids[i])) onReorder(order);
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
    itemProps: (id) => ({
      ref: (el) => {
        if (!el) return;
        elements.current.set(id, el);
        return () => {
          elements.current.delete(id);
        };
      },
      onPointerDown: (e) => start(id, e),
    }),
  };
}
