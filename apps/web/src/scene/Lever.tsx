import type { UIProps } from '../design/index';

import css from './Lever.module.css';
export type LeverProps = Omit<UIProps, 'className' | 'style'> & {
  levels: string[];
  value: number;
  onChange: (value: number) => void;
};
export function Lever({ levels, value, onChange, ...rest }: LeverProps) {
  return (
    <input
      {...rest}
      className={css.root}
      type="range"
      min={0}
      max={Math.max(0, levels.length - 1)}
      step={1}
      value={value}
      aria-valuetext={levels[value]}
      onChange={(e) => onChange(Number(e.target.value))}
    />
  );
}
