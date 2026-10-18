import type { UIProps, Status } from './types';
import { useState, type Ref } from 'react';
import { StatusDot } from './StatusDot';
import { Icon } from './Icon';
import css from './Avatar.module.css';
export type AvatarProps = Omit<UIProps, 'className' | 'style'> & {
  ref?: Ref<HTMLButtonElement | HTMLSpanElement>;
  src?: string;
  fallback?: string;
  size: 20 | 22 | 26 | 32 | 40 | 44 | 60 | 76 | 88 | 'fill';
  status?: Status;
  statusTip?: string;
  dimmed?: boolean;
  creating?: boolean;
  selected?: boolean;
  onEdit?: () => void;
  editLabel?: string;
};
// A picture that cannot be read gives way to the fallback, else the plain placeholder; each src starts afresh.
function AvatarImage({
  src,
  fallback,
  size,
}: {
  src: string;
  fallback?: string;
  size: AvatarProps['size'];
}) {
  const [failed, setFailed] = useState(false);
  const shown = failed ? fallback : src;
  if (!shown) return null;
  return (
    <img
      key={shown}
      src={shown}
      onError={() => !failed && setFailed(true)}
      width={size === 'fill' ? undefined : size}
      height={size === 'fill' ? undefined : size}
      alt=""
    />
  );
}
export function Avatar({
  ref,
  src,
  fallback,
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
      {src && <AvatarImage key={src} src={src} fallback={fallback} size={size} />}
      {status && <StatusDot status={status} tip={statusTip} />}
      {onEdit && (
        <span className={css.edit}>
          <Icon name="pencil" size={size === 'fill' || size >= 60 ? 20 : 14} />
        </span>
      )}
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
