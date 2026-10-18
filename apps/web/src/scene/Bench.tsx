import type { ReactNode } from 'react';
import type { UIProps } from '../design/index';
import css from './Bench.module.css';
export type BenchProps = Omit<UIProps, 'className' | 'style'> & { children: ReactNode };
// The console on the left, the badge hanging beside it; a narrow window hangs it below.
export function Bench({ children, ...rest }: BenchProps) {
  return (
    <div {...rest} className={css.root}>
      {children}
    </div>
  );
}
