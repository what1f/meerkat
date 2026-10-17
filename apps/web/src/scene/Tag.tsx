import type { ReactNode } from 'react';
import type { UIProps } from '../design/index';
import peg from './peg.module.css';
import css from './Tag.module.css';
export type TagProps = Omit<UIProps, 'className' | 'style'> & {
  children: ReactNode;
  tone?: 'paper';
};
// A shipping tag on a string, written on like paper.
export function Tag({ children, tone = 'paper', ...rest }: TagProps) {
  return (
    <section {...rest} className={`${peg.hang} ${css.root}`} data-tone={tone}>
      <span className={peg.peg} data-peg="" />
      <span className={peg.cord} />
      <div className={peg.shadow}>
        <div className={css.tag}>
          <span className={peg.eyelet} />
          {children}
        </div>
      </div>
    </section>
  );
}
