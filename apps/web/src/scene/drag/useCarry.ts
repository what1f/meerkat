import type { PointerEventHandler, RefCallback } from 'react';
export type CarryHandle = {
  ref: RefCallback<HTMLElement>;
  onPointerDown: PointerEventHandler<HTMLElement>;
};
export function useCarry(onDrop: (id: string, rowKey: string) => void): {
  seatHandle: (id: string) => CarryHandle;
  rowProps: (rowKey: string) => { ref: RefCallback<HTMLElement> };
} {
  void onDrop;
  return {
    seatHandle: () => ({ ref: () => {}, onPointerDown: () => {} }),
    rowProps: () => ({ ref: () => {} }),
  };
}
