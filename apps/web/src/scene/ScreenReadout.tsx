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
      <div>
        {ai.mark}
        {ai.label}
      </div>
      <div>{model}</div>
      <div>
        <StatusDot status={place.status} />
        {place.label}
      </div>
      <meter min={0} max={level.total} value={level.index} aria-label={level.label} />
    </div>
  );
}
