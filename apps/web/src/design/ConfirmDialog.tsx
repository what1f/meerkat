import type { ReactNode } from 'react';
import type { UIProps } from './types';
import { Dialog } from './Dialog';
import { Button } from './Button';
import css from './ConfirmDialog.module.css';
export type ConfirmDialogProps = Omit<UIProps, 'className' | 'style'> & {
  open: boolean;
  onClose: () => void;
  leading?: ReactNode;
  title: string;
  body: ReactNode;
  confirmLabel: string;
  cancelLabel: string;
  onConfirm: () => void;
  tone?: 'danger';
  /** The confirmed action is in flight; the confirm button waits for it. */
  pending?: boolean;
};
export function ConfirmDialog({
  open,
  onClose,
  leading,
  title,
  body,
  confirmLabel,
  cancelLabel,
  onConfirm,
  tone,
  pending,
  ...rest
}: ConfirmDialogProps) {
  return (
    <Dialog
      {...rest}
      data-confirm
      size="sm"
      open={open}
      onClose={onClose}
      title={title}
      footer={
        <div className={css.root}>
          <Button onClick={onClose}>{cancelLabel}</Button>
          <Button
            variant={tone === 'danger' ? 'danger' : 'primary'}
            loading={pending}
            onClick={onConfirm}
          >
            {confirmLabel}
          </Button>
        </div>
      }
    >
      {leading && <div className={css.leading}>{leading}</div>}
      {body}
    </Dialog>
  );
}
