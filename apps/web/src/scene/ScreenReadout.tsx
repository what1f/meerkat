import type { ReactNode } from 'react';
import type { UIProps, Status } from '../design/index';
import { StatusDot } from '../design';
import css from './ScreenReadout.module.css';
export type ScreenReadoutProps = Omit<UIProps, 'className' | 'style'> & {
  ai: { mark: ReactNode; label: string };
  model: string;
  place: { label: string; status: Status };
  level: { index: number; total: number; label: string };
};
export function ScreenReadout({ ai, model, place, level, ...rest }: ScreenReadoutProps) {
  return (
    <div {...rest} className={css.root}>
      <div className={css.ai}>
        {ai.mark}
        {ai.label}
      </div>
      <div className={css.model}>{model}</div>
      <div className={css.where}>
        <StatusDot status={place.status} />
        {place.label}
      </div>
      <div
        className={css.gauge}
        role="meter"
        aria-valuemin={0}
        aria-valuemax={Math.max(0, level.total - 1)}
        aria-valuenow={level.index}
        aria-valuetext={level.label}
      >
        {Array.from({ length: level.total }, (_, i) => (
          <i key={i} data-on={i <= level.index} />
        ))}
        <b>{level.label}</b>
      </div>
    </div>
  );
}
