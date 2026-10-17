import type { CSSProperties, RefCallback } from 'react';
export type HolePosition = { x: number; y: number };
export type HoleItem = { key: string; kind: string; pos: HolePosition };
export type HoleHandle = { ref: RefCallback<HTMLElement>; style: CSSProperties; dragging: boolean };
export function useHoleDrag(
  items: HoleItem[],
  onMove: (key: string, pos: HolePosition) => void,
): Record<string, HoleHandle> {
  void onMove;
  return Object.fromEntries(
    items.map((item) => [item.key, { ref: () => {}, style: {}, dragging: false }]),
  );
}
