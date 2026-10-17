import type { ReactNode } from 'react';
import type { UIProps } from '../design/index';
import peg from './peg.module.css';
import css from './Clipboard.module.css';
export type ClipboardProps = Omit<UIProps, 'className' | 'style'> & {
  title: ReactNode;
  actions: ReactNode;
  children: ReactNode;
  footer: ReactNode;
};
// A clipboard on a peg: ruled paper under a clamp, a second sheet underneath.
export function Clipboard({ title, actions, children, footer, ...rest }: ClipboardProps) {
  return (
    <section {...rest} className={css.root}>
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
