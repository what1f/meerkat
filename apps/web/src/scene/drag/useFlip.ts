import { useCallback } from 'react';
const flip = { duration: 560, easing: 'cubic-bezier(.22, 1, .36, 1)' };
// Play an element in from where it was: translate and scale from the old rect to its own.
export function useFlip(): (from: DOMRect, el: HTMLElement) => void {
  return useCallback((from: DOMRect, el: HTMLElement) => {
    const to = el.getBoundingClientRect();
    if (!to.width || !to.height) return;
    const dx = from.left - to.left;
    const dy = from.top - to.top;
    el.animate(
      [
        {
          transformOrigin: 'top left',
          transform: `translate(${dx}px, ${dy}px) scale(${from.width / to.width})`,
        },
        { transformOrigin: 'top left', transform: 'none' },
      ],
      flip,
    );
  }, []);
}
