import type { ReactNode } from 'react';
import type { UIProps } from './types';

import css from './Callout.module.css';
export type CalloutProps = Omit<UIProps, 'className' | 'style'> & {
  tone: 'quiet' | 'attention';
  leading?: ReactNode;
  children: ReactNode;
  action?: ReactNode;
};
export function Callout({ tone, leading, children, action, ...rest }: CalloutProps) {
  return (
    <aside {...rest} className={css.root} data-tone={tone}>
      {leading}
      <div>{children}</div>
      {action}
    </aside>
  );
}
