import type { ReactNode } from 'react';
import type { UIProps } from './types';
import { useEffect, useRef } from 'react';
import { Layer } from './layer';
import css from './Closeup.module.css';
export type CloseupProps = Omit<UIProps, 'className' | 'style'> & {
  open: boolean;
  onClose: () => void;
  /** Names the dialog; the object itself carries no window chrome. */
  label: string;
  children: ReactNode;
};
// A modal without chrome: the object itself is held up over the dimmed page. Esc or a click
// outside puts it back.
export function Closeup({ open, onClose, label, children, ...rest }: CloseupProps) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (open && !dialog.open) {
      dialog.showModal();
      dialog.focus();
    }
    if (!open && dialog.open) dialog.close();
  }, [open]);
  return (
    <dialog
      {...rest}
      ref={ref}
      className={css.root}
      tabIndex={-1}
      aria-label={label}
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <Layer value={ref}>
        <div className={css.object}>{children}</div>
      </Layer>
    </dialog>
  );
}
