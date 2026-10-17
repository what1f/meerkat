import type { ReactNode } from 'react';
import type { UIProps } from '../design/index';

import css from './Board.module.css';
export type BoardProps = Omit<UIProps, 'className' | 'style'> & {
  layout: 'rails' | 'free';
  holes?: 'on' | 'fade';
  children: ReactNode;
};
export function Board({ layout, holes = 'on', children, ...rest }: BoardProps) {
  return (
    <section {...rest} className={css.root} data-layout={layout} data-holes={holes}>
      {children}
    </section>
  );
}
