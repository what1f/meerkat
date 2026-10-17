import type { ReactNode } from 'react';
import type { UIProps } from '../design/index';

import css from './Tag.module.css';
export type TagProps = Omit<UIProps, 'className' | 'style'> & {
  children: ReactNode;
  tone?: 'paper';
};
export function Tag({ children, tone = 'paper', ...rest }: TagProps) {
  return (
    <section {...rest} className={css.root} data-tone={tone}>
      {children}
    </section>
  );
}
