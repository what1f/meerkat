import type { Ref } from 'react';
import type { ReactNode } from 'react';
import type { UIProps } from '../design/index';
import { Icon, Tooltip, type IconName } from '../design';
import css from './ConsoleKey.module.css';
export type ConsoleKeyProps = Omit<UIProps, 'className' | 'style'> & {
  ref?: Ref<HTMLButtonElement>;
  icon?: IconName;
  label: string;
  lit?: boolean;
  go?: boolean;
  disabled?: boolean;
  onClick: () => void;
  children?: ReactNode;
};
export function ConsoleKey({
  icon,
  label,
  lit,
  go,
  disabled,
  onClick,
  children,
  ...rest
}: ConsoleKeyProps) {
  const key = (
    <button
      {...rest}
      className={css.root}
      type="button"
      data-lit={lit}
      data-go={go}
      disabled={disabled}
      onClick={onClick}
      aria-label={label}
    >
      {go && <i className={css.led} />}
      {icon && <Icon name={icon} size={16} />}
      {children ?? (icon ? null : label)}
    </button>
  );
  // Icon keys name themselves on hover.
  return icon && !children ? <Tooltip label={label}>{key}</Tooltip> : key;
}
