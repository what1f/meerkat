import type { UIProps } from './types';

import css from './Spinner.module.css';
export type SpinnerProps = Omit<UIProps, 'className' | 'style'> & { size?: 'sm' | 'md' };
export function Spinner({ size = 'sm', ...rest }: SpinnerProps) {
  return <span {...rest} className={css.root} data-size={size} aria-busy="true" />;
}
