import type { UIProps } from './types';

import css from './Dashes.module.css';
export type DashesProps = Omit<UIProps, 'className' | 'style'> & { total: number; current: number };
export function Dashes({ total, current, ...rest }: DashesProps) {
  return (
    <ol {...rest} className={css.root}>
      {Array.from({ length: Math.max(0, total) }, (_, i) => (
        <li key={i} aria-current={i === current ? 'step' : undefined} />
      ))}
    </ol>
  );
}
