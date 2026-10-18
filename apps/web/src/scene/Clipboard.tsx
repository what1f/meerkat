import type { ReactNode, Ref } from 'react';
import type { UIProps } from '../design/index';
import type { PegPosition } from './Hang';
import peg from './peg.module.css';
import css from './Clipboard.module.css';
export type ClipboardProps = Omit<UIProps, 'className' | 'style'> & {
  ref?: Ref<HTMLElement>;
  /** Where the peg sits on a free board. */
  style?: PegPosition;
  title: ReactNode;
  actions: ReactNode;
  children: ReactNode;
  footer: ReactNode;
};
// A clipboard on a peg: ruled paper under a clamp, a second sheet underneath.
export function Clipboard({
  title,
  actions,
  children,
  footer,
  ref,
  style,
  ...rest
}: ClipboardProps) {
  return (
    <section {...rest} ref={ref} style={style} className={css.root}>
      <span className={peg.peg} data-peg="" />
      <div className={css.wood}>
        <span className={css.under} />
        <span className={css.clamp} />
        <div className={css.paper}>
          <header>
            <div className={css.title}>{title}</div>
            {actions}
          </header>
          <div>{children}</div>
          {footer}
          <span className={css.lines} />
        </div>
      </div>
    </section>
  );
}
