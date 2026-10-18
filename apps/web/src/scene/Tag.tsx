import type { ReactNode, Ref } from 'react';
import type { UIProps } from '../design/index';
import type { PegPosition } from './Hang';
import peg from './peg.module.css';
import css from './Tag.module.css';
export type TagProps = Omit<UIProps, 'className' | 'style'> & {
  ref?: Ref<HTMLElement>;
  /** Where the peg sits on a free board. */
  style?: PegPosition;
  children: ReactNode;
  tone?: 'paper';
};
// A shipping tag on a string, written on like paper.
export function Tag({ children, tone = 'paper', ref, style, ...rest }: TagProps) {
  return (
    <section
      {...rest}
      ref={ref}
      style={style}
      className={`${peg.hang} ${css.root}`}
      data-tone={tone}
    >
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
