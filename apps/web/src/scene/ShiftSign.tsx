import type { UIProps } from '../design/index';

import css from './ShiftSign.module.css';
export type ShiftSignProps = Omit<UIProps, 'className' | 'style'> & {
  on: boolean;
  onToggle: () => void;
  onLabel: string;
  offLabel: string;
  label: string;
};
export function ShiftSign({ on, onToggle, onLabel, offLabel, label, ...rest }: ShiftSignProps) {
  return (
    <button
      {...rest}
      className={css.root}
      type="button"
      role="switch"
      aria-checked={on}
      aria-label={label}
      onClick={onToggle}
    >
      {on ? onLabel : offLabel}
    </button>
  );
}
