import type { UIProps, Status } from './types';
import type { Ref } from 'react';
import { StatusDot } from './StatusDot';
import { Icon } from './Icon';
import css from './Avatar.module.css';
export type AvatarProps = Omit<UIProps, 'className' | 'style'> & {
  ref?: Ref<HTMLButtonElement | HTMLSpanElement>;
  src?: string;
  size: 20 | 26 | 32 | 40 | 44 | 60 | 76 | 88 | 'fill';
  status?: Status;
  statusTip?: string;
  dimmed?: boolean;
  creating?: boolean;
  selected?: boolean;
  onEdit?: () => void;
  editLabel?: string;
};
export function Avatar({
  ref,
  src,
  size,
  status,
  statusTip,
  dimmed,
  creating,
  selected,
  onEdit,
  editLabel,
  ...rest
}: AvatarProps) {
  const content = (
    <>
      {src && (
        <img
          src={src}
          width={size === 'fill' ? undefined : size}
          height={size === 'fill' ? undefined : size}
          alt=""
        />
      )}
      {status && <StatusDot status={status} tip={statusTip} />}
      {onEdit && <Icon name="pencil" />}
    </>
  );
  const state = {
    'data-size': size,
    'data-dimmed': dimmed,
    'data-selected': selected,
    'aria-busy': creating,
  };
  return onEdit ? (
    <button
      {...rest}
      {...state}
      ref={ref as Ref<HTMLButtonElement>}
      className={css.root}
      type="button"
      aria-label={rest['aria-label'] ?? editLabel}
      title={editLabel}
      onClick={(e) => {
        rest.onClick?.(e);
        if (!e.defaultPrevented) onEdit();
      }}
    >
      {content}
    </button>
  ) : (
    <span {...rest} {...state} ref={ref as Ref<HTMLSpanElement>} className={css.root}>
      {content}
    </span>
  );
}
