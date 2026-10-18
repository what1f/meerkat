import { useRef, useSyncExternalStore, type DOMAttributes, type PointerEvent } from 'react';
import { after } from '../lib/after';
import { readStorage, writeStorage } from '../lib/storage';
export type SidebarLayout = { width: number; rail: boolean; hidden: boolean };
// Drag limits from the design: past FOLD the sidebar keeps only its icons.
export const SIDEBAR_MIN = 180;
export const SIDEBAR_MAX = 320;
export const SIDEBAR_FOLD = 132;
export const SIDEBAR_RAIL = 84;
const SETTLE_MS = 400;
const defaults: SidebarLayout = { width: 220, rail: false, hidden: false };
function valid(value: unknown): value is SidebarLayout {
  if (!value || typeof value !== 'object') return false;
  const v = value as Partial<SidebarLayout>;
  return (
    typeof v.width === 'number' &&
    Number.isFinite(v.width) &&
    v.width >= SIDEBAR_MIN &&
    v.width <= SIDEBAR_MAX &&
    typeof v.rail === 'boolean' &&
    typeof v.hidden === 'boolean'
  );
}
let saved = readStorage('sidebar', defaults, valid);
// Transient: the settle transition runs only while the sidebar changes form, never while dragging.
let view = { ...saved, settling: false, resizing: false };
let cancelSettle: (() => void) | undefined;
const listeners = new Set<() => void>();
function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
function publish(patch: Partial<typeof view>) {
  view = { ...view, ...patch };
  saved = { width: view.width, rail: view.rail, hidden: view.hidden };
  writeStorage('sidebar', saved);
  listeners.forEach((fn) => fn());
}
function settle() {
  cancelSettle?.();
  view = { ...view, settling: true };
  cancelSettle = after(SETTLE_MS, () => {
    view = { ...view, settling: false };
    listeners.forEach((fn) => fn());
  });
}
export function setSidebarLayout(patch: Partial<SidebarLayout>) {
  const width = Math.max(SIDEBAR_MIN, Math.min(SIDEBAR_MAX, patch.width ?? view.width));
  if (
    (patch.rail !== undefined && patch.rail !== view.rail) ||
    (patch.hidden !== undefined && patch.hidden !== view.hidden)
  )
    settle();
  publish({ ...patch, width });
}
function setResizing(resizing: boolean) {
  view = { ...view, resizing };
  listeners.forEach((fn) => fn());
}
export function useSidebarLayout() {
  const layout = useSyncExternalStore(subscribe, () => view);
  return {
    ...layout,
    setWidth: (width: number) => setSidebarLayout({ width }),
    setRail: (rail: boolean) => setSidebarLayout({ rail }),
    setHidden: (hidden: boolean) => setSidebarLayout({ hidden }),
  };
}

export function useSidebarGrip() {
  const { width, rail, setRail } = useSidebarLayout();
  const drag = useRef<{ x: number; width: number } | null>(null);
  const end = (e: PointerEvent<HTMLDivElement>) => {
    drag.current = null;
    setResizing(false);
    if (e.currentTarget.hasPointerCapture(e.pointerId))
      e.currentTarget.releasePointerCapture(e.pointerId);
  };
  const handlers: DOMAttributes<HTMLDivElement> = {
    onDoubleClick: () => setRail(!rail),
    onKeyDown: (e) => {
      if (e.key === 'Enter') {
        e.preventDefault();
        setRail(!rail);
      }
      if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
        e.preventDefault();
        setSidebarLayout({ rail: false, width: width + (e.key === 'ArrowRight' ? 16 : -16) });
      }
    },
    onPointerDown: (e) => {
      if (e.button !== 0) return;
      drag.current = { x: e.clientX, width: rail ? SIDEBAR_RAIL : width };
      e.currentTarget.setPointerCapture(e.pointerId);
      setResizing(true);
    },
    onPointerMove: (e) => {
      if (!drag.current) return;
      const next = drag.current.width + e.clientX - drag.current.x;
      // Below the fold only the rail remains; the remembered width stays for the way back.
      if (next < SIDEBAR_FOLD) setSidebarLayout({ rail: true });
      else setSidebarLayout({ rail: false, width: next });
    },
    onPointerUp: end,
    onPointerCancel: end,
  };
  return { width, handlers };
}
