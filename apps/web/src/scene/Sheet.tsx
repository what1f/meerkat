import { useEffect, useRef, useState, type ReactNode } from 'react';
import css from './Sheet.module.css';

export type SheetClose = {
  onClose: () => void;
  // Called instead of closing, e.g. while there are unsaved edits.
  blocked?: () => void;
};
export type SheetProps = { close?: SheetClose; children: ReactNode };

// Mark an element whose own background counts as empty space around the sheet.
export const blank = { 'data-blank': '' };

// A page laid over the one it was opened from, which stays underneath.
// A click on empty space or Esc puts it back.
export function Sheet({ close, children }: SheetProps) {
  const ref = useRef<HTMLDivElement>(null);
  const down = useRef(false);
  const [leaving, setLeaving] = useState(false);
  const latest = useRef(close);
  useEffect(() => {
    latest.current = close;
  });
  // An open menu, popover or dialog takes Esc and outside clicks for itself first.
  // Tooltips do not: one is often up under the pointer that just opened the sheet.
  const busy = () =>
    !!document.querySelector(
      'dialog[open], [data-open]:is([role="menu"], [role="dialog"], [role="listbox"])',
    );
  const dismiss = () => {
    const c = latest.current;
    if (!c || leaving) return;
    if (c.blocked) return c.blocked();
    if (matchMedia('(prefers-reduced-motion: reduce)').matches) return c.onClose();
    setLeaving(true);
  };
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || e.defaultPrevented || busy()) return;
      e.preventDefault();
      dismiss();
    };
    addEventListener('keydown', onKey);
    return () => removeEventListener('keydown', onKey);
  });
  const isBlank = (t: EventTarget) => t instanceof HTMLElement && t.hasAttribute('data-blank');
  return (
    <div
      ref={ref}
      className={css.root}
      data-testid="sheet"
      data-leaving={leaving || undefined}
      {...blank}
      onAnimationEnd={(e) => {
        if (leaving && e.target === ref.current) latest.current?.onClose();
      }}
      // Both ends of the click must land on empty space, so a text selection
      // dragged out of a field does not put the sheet away.
      onPointerDown={(e) => {
        down.current = isBlank(e.target) && !busy();
      }}
      onClick={(e) => {
        if (down.current && isBlank(e.target)) dismiss();
        down.current = false;
      }}
    >
      {children}
    </div>
  );
}
