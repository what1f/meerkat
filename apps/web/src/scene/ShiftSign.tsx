import { useState, type Ref } from 'react';
import type { UIProps } from '../design/index';
import type { PegPosition } from './Hang';
import css from './ShiftSign.module.css';
export type ShiftSignProps = Omit<UIProps, 'className' | 'style'> & {
  ref?: Ref<HTMLDivElement>;
  /** Where the peg sits on a free board. */
  style?: PegPosition;
  on: boolean;
  onToggle: () => void;
  onLabel: string;
  offLabel: string;
  label: string;
};
// A two-sided sign on a peg. Turning it swings it on its strings.
export function ShiftSign({
  on,
  onToggle,
  onLabel,
  offLabel,
  label,
  ref,
  style,
  ...rest
}: ShiftSignProps) {
  const [turns, setTurns] = useState(0);
  return (
    <div
      {...rest}
      ref={ref}
      style={style}
      className={css.root}
      data-on={on}
      data-turn={turns ? turns % 2 : undefined}
    >
      <span className={css.peg} data-peg="" />
      <svg className={css.strings} viewBox="0 0 120 48" aria-hidden="true">
        <polyline points="17,50 60,12 103,50" />
      </svg>
      <button
        className={css.sign}
        type="button"
        role="switch"
        aria-checked={on}
        aria-label={label}
        onClick={() => {
          setTurns(turns + 1);
          onToggle();
        }}
      >
        <span className={css.face}>{onLabel}</span>
        <span className={css.face}>{offLabel}</span>
      </button>
    </div>
  );
}
