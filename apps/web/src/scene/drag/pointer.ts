import { useEffect, useRef } from 'react';
// Shared pointer helpers for the board drags.
export const threshold = 5;
export const clamp = (v: number, min: number, max: number) => Math.max(min, Math.min(max, v));
// The item trails the hand like a pendulum.
export const trail = (tilt: number, dx: number) => tilt + (clamp(dx * 1.2, -9, 9) - tilt) * 0.3;
export function grabbing(on: boolean) {
  if (on) document.body.dataset.grabbing = '';
  else delete document.body.dataset.grabbing;
}
// Swallow the click that ends a drag so the item does not also open.
export function eatClick() {
  const eat = (e: MouseEvent) => {
    e.stopPropagation();
    e.preventDefault();
  };
  window.addEventListener('click', eat, { capture: true, once: true });
  setTimeout(() => window.removeEventListener('click', eat, { capture: true }), 0);
}
// Restart the landing swing: the attribute is styled by the board.
export function swing(el: HTMLElement) {
  delete el.dataset.swing;
  void el.offsetWidth;
  el.dataset.swing = '';
}
export type Follow = {
  move: (e: PointerEvent) => void;
  /** The pointer was released: commit the drop. */
  up: (e: PointerEvent) => void;
  /** The gesture was taken away (touch cancel, lost capture): restore, never commit. */
  cancel: () => void;
};
export type Tracking = {
  /** Capture the pointer once the drag really starts; false if the item is gone. */
  capture: () => boolean;
  /** Stop listening without calling back. Safe to call more than once. */
  stop: () => void;
};
// Follow one pointer until it is released or cancelled.
export function follow(e: { pointerId: number }, el: Element, h: Follow): Tracking {
  const id = e.pointerId;
  let done = false;
  let held = false;
  const stop = () => {
    if (done) return;
    done = true;
    window.removeEventListener('pointermove', onMove);
    window.removeEventListener('pointerup', onUp);
    window.removeEventListener('pointercancel', onCancel);
    window.removeEventListener('lostpointercapture', onLost);
    if (el.isConnected && el.hasPointerCapture(id)) el.releasePointerCapture(id);
  };
  const onMove = (ev: PointerEvent) => ev.pointerId === id && h.move(ev);
  const onUp = (ev: PointerEvent) => {
    if (ev.pointerId !== id) return;
    stop();
    h.up(ev);
  };
  const onCancel = (ev: PointerEvent) => {
    if (ev.pointerId !== id) return;
    stop();
    h.cancel();
  };
  // Taking capture from a touch's implicit target also fires this; only our own loss counts.
  // A removed item loses capture at the document instead of on itself.
  const onLost = (ev: PointerEvent) => {
    if (held && (ev.target === el || !el.isConnected)) onCancel(ev);
  };
  window.addEventListener('pointermove', onMove);
  window.addEventListener('pointerup', onUp);
  window.addEventListener('pointercancel', onCancel);
  window.addEventListener('lostpointercapture', onLost);
  const capture = () => {
    if (done || !el.isConnected) return false;
    try {
      el.setPointerCapture(id);
      held = true;
      return true;
    } catch {
      return false;
    }
  };
  return { capture, stop };
}
// One drag at a time per hook. Unmounting aborts the drag without committing it
// and runs pending settle steps at once, so nothing is left on the page.
export function useDragLife() {
  const life = useRef<{
    abort: (() => void) | null;
    pending: Map<ReturnType<typeof setTimeout>, () => void>;
  }>({ abort: null, pending: new Map() });
  useEffect(() => {
    const l = life.current;
    return () => {
      l.abort?.();
      l.abort = null;
      l.pending.forEach((fn, timer) => {
        clearTimeout(timer);
        fn();
      });
      l.pending.clear();
    };
  }, []);
  return {
    /** Register how to undo the drag in progress; replaces any previous one. */
    begin: (abort: () => void) => {
      life.current.abort?.();
      life.current.abort = abort;
    },
    end: () => {
      life.current.abort = null;
    },
    later: (fn: () => void, ms: number) => {
      const timer = setTimeout(() => {
        life.current.pending.delete(timer);
        fn();
      }, ms);
      life.current.pending.set(timer, fn);
    },
  };
}
