import type { ReactNode } from 'react';
import type { UIProps } from './types';
import { useEffect, useId, useRef } from 'react';
import css from './Dialog.module.css';
export type DialogProps = Omit<UIProps, 'className' | 'style'> & {
  open: boolean;
  onClose: () => void;
  title: string;
  size?: 'sm' | 'md';
  children: ReactNode;
  footer: ReactNode;
};
export function Dialog({
  open,
  onClose,
  title,
  size = 'md',
  children,
  footer,
  ...rest
}: DialogProps) {
  const ref = useRef<HTMLDialogElement>(null);
  const id = useId();
  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (open && !dialog.open) dialog.showModal();
    if (!open && dialog.open) dialog.close();
  }, [open]);
  return (
    <dialog
      {...rest}
      ref={ref}
      className={css.root}
      data-size={size}
      aria-labelledby={id}
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
      onClick={(e) => {
        if (e.target !== e.currentTarget) return;
        const box = e.currentTarget.getBoundingClientRect();
        if (
          e.clientX < box.left ||
          e.clientX > box.right ||
          e.clientY < box.top ||
          e.clientY > box.bottom
        )
          onClose();
      }}
    >
      <header>
        <h2 id={id}>{title}</h2>
      </header>
      <div>{children}</div>
      <footer>{footer}</footer>
    </dialog>
  );
}
