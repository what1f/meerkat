import { useRef, useSyncExternalStore, type DOMAttributes } from 'react';
import { readStorage, writeStorage } from '../lib/storage';
export type SidebarLayout = { width: number; rail: boolean; hidden: boolean };
const defaults: SidebarLayout = { width: 240, rail: false, hidden: false };
function valid(value: unknown): value is SidebarLayout {
  if (!value || typeof value !== 'object') return false;
  const v = value as Partial<SidebarLayout>;
  return (
    typeof v.width === 'number' &&
    Number.isFinite(v.width) &&
    v.width >= 160 &&
    v.width <= 480 &&
    typeof v.rail === 'boolean' &&
    typeof v.hidden === 'boolean'
  );
}
let state = readStorage('sidebar', defaults, valid);
const listeners = new Set<() => void>();
function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
export function setSidebarLayout(patch: Partial<SidebarLayout>) {
  state = { ...state, ...patch, width: Math.max(160, Math.min(480, patch.width ?? state.width)) };
  writeStorage('sidebar', state);
  listeners.forEach((fn) => fn());
}
export function useSidebarLayout() {
  const layout = useSyncExternalStore(subscribe, () => state);
  return {
    ...layout,
    setWidth: (width: number) => setSidebarLayout({ width }),
    setRail: (rail: boolean) => setSidebarLayout({ rail }),
    setHidden: (hidden: boolean) => setSidebarLayout({ hidden }),
  };
}

export function useSidebarGrip() {
  const { width, rail, setWidth, setRail } = useSidebarLayout();
  const drag = useRef<{ x: number; width: number } | null>(null);
  const handlers: DOMAttributes<HTMLDivElement> = {
    onDoubleClick: () => setRail(!rail),
    onKeyDown: (e) => {
      if (e.key === 'Enter') {
        e.preventDefault();
        setRail(!rail);
      }
      if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
        e.preventDefault();
        setRail(false);
        setWidth(width + (e.key === 'ArrowRight' ? 16 : -16));
      }
    },
    onPointerDown: (e) => {
      if (e.button !== 0) return;
      drag.current = { x: e.clientX, width };
      e.currentTarget.setPointerCapture(e.pointerId);
    },
    onPointerMove: (e) => {
      if (drag.current) {
        setRail(false);
        setWidth(drag.current.width + e.clientX - drag.current.x);
      }
    },
    onPointerUp: (e) => {
      drag.current = null;
      if (e.currentTarget.hasPointerCapture(e.pointerId))
        e.currentTarget.releasePointerCapture(e.pointerId);
    },
    onPointerCancel: () => {
      drag.current = null;
    },
  };
  return { width, handlers };
}
