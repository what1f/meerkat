import { useState } from 'react';
import type { UIProps } from '../design/index';
import css from './ShiftSign.module.css';
export type ShiftSignProps = Omit<UIProps, 'className' | 'style'> & {
  on: boolean;
  onToggle: () => void;
  onLabel: string;
  offLabel: string;
  label: string;
};
// A two-sided sign on a peg. Turning it swings it on its strings.
export function ShiftSign({ on, onToggle, onLabel, offLabel, label, ...rest }: ShiftSignProps) {
  const [turns, setTurns] = useState(0);
  return (
    <div {...rest} className={css.root} data-on={on} data-turn={turns ? turns % 2 : undefined}>
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
