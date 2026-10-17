import type { ReactNode } from 'react';
import type { UIProps } from '../design/index';

import css from './Clipboard.module.css';
export type ClipboardProps = Omit<UIProps, 'className' | 'style'> & {
  title: ReactNode;
  actions: ReactNode;
  children: ReactNode;
  footer: ReactNode;
};
export function Clipboard({ title, actions, children, footer, ...rest }: ClipboardProps) {
  return (
    <section {...rest} className={css.root}>
      <header>
        {title}
        {actions}
      </header>
      <div>{children}</div>
      <footer>{footer}</footer>
    </section>
  );
}
