import type { PointerEventHandler, RefCallback } from 'react';
export type SortableHandle = {
  ref: RefCallback<HTMLElement>;
  onPointerDown: PointerEventHandler<HTMLElement>;
};
export function useSortable(
  ids: string[],
  onReorder: (ids: string[]) => void,
): { itemProps: (id: string) => SortableHandle } {
  void ids;
  void onReorder;
  return { itemProps: () => ({ ref: () => {}, onPointerDown: () => {} }) };
}
