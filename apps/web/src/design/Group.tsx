import type { ReactNode } from 'react';
import type { UIProps } from './types';

import css from './Group.module.css';
export type GroupProps = Omit<UIProps, 'className' | 'style'> & { children: ReactNode };
export function Group({ children, ...rest }: GroupProps) {
  return (
    <div {...rest} className={css.root}>
      {children}
    </div>
  );
}
export type GroupRowProps = Omit<UIProps, 'className' | 'style'> & {
  label: ReactNode;
  children: ReactNode;
  align?: 'center' | 'top';
};
Group.Row = function Row({ label, children, align = 'center', ...rest }: GroupRowProps) {
  return (
    <div {...rest} className={css.root} data-align={align}>
      <div>{label}</div>
      <div>{children}</div>
    </div>
  );
};
