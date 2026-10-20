import type { ReactNode } from 'react';
import type { UIProps } from './types';

import css from './Reveal.module.css';
export type RevealProps = Omit<UIProps, 'className' | 'style'> & {
  open: boolean;
  children: ReactNode;
};
export function Reveal({ open, children, ...rest }: RevealProps) {
  return (
    <div {...rest} className={css.root} data-open={open}>
      <div inert={!open}>{children}</div>
    </div>
  );
}
