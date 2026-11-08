import { useState, type PointerEvent } from 'react';
import type { UIProps } from '../design/index';
import css from './Lever.module.css';
export type LeverProps = Omit<UIProps, 'className' | 'style'> & {
  levels: string[];
  value: number;
  onChange: (value: number) => void;
};
// A knob in a notched vertical gate; the top notch is the highest level.
const top = 10;
const travel = 166;
export function Lever({ levels, value, onChange, ...rest }: LeverProps) {
  const [held, setHeld] = useState(false);
  const last = Math.max(0, levels.length - 1);
  const y = (i: number) => top + travel * (last ? 1 - i / last : 0.5);
  const set = (i: number) => {
    const next = Math.max(0, Math.min(last, i));
    if (next !== value) onChange(next);
  };
  const nearest = (e: PointerEvent<HTMLElement>) => {
    const t = (e.clientY - e.currentTarget.getBoundingClientRect().top - top) / travel;
    return Math.round((1 - t) * last);
  };
  const step = { ArrowUp: 1, ArrowRight: 1, ArrowDown: -1, ArrowLeft: -1 } as const;
  return (
    <div
      {...rest}
      className={css.root}
      role="slider"
      tabIndex={0}
      aria-orientation="vertical"
      aria-valuemin={0}
      aria-valuemax={last}
      aria-valuenow={value}
      aria-valuetext={levels[value]}
      data-held={held}
      onPointerDown={(e) => {
        // Keep the drag from selecting the level labels; focus still moves here.
        e.preventDefault();
        e.currentTarget.focus();
        e.currentTarget.setPointerCapture(e.pointerId);
        setHeld(true);
        set(nearest(e));
      }}
      onPointerMove={(e) => held && set(nearest(e))}
      onPointerUp={() => setHeld(false)}
      onPointerCancel={() => setHeld(false)}
      onKeyDown={(e) => {
        const by = step[e.key as keyof typeof step];
        const to =
          e.key === 'Home' ? 0 : e.key === 'End' ? last : by === undefined ? -1 : value + by;
        if (to < 0 && by === undefined) return;
        e.preventDefault();
        set(to);
      }}
    >
      <span className={css.gate} />
      <span className={css.fill} style={{ top: y(value) }} />
      {levels.map((level, i) => (
        <span
          key={i}
          className={css.notch}
          data-on={i === value}
          data-below={i < value}
          style={{ top: y(i) }}
        >
          {level}
        </span>
      ))}
      <span className={css.knob} style={{ top: y(value) }} />
    </div>
  );
}
