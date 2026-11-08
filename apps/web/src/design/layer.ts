import { createContext, useContext, type RefObject } from 'react';

// A modal <dialog> sits in the browser's top layer, above anything portaled to <body>.
// Popups opened inside one portal into the dialog instead, and position against the viewport.
export const Layer = createContext<RefObject<HTMLElement | null> | null>(null);

export function useLayer() {
  const container = useContext(Layer);
  return container
    ? ({ container, positionMethod: 'fixed' } as const)
    : ({ container: undefined, positionMethod: undefined } as const);
}
