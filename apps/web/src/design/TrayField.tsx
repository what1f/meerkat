import type { ReactNode } from 'react';
import type { UIProps } from './types';

import css from './TrayField.module.css';
export type TrayFieldProps = Omit<UIProps, 'className' | 'style'> & {
  label: string;
  children: ReactNode;
};
// A small bold label above full-width inputs, for forms laid straight on the paper tray.
export function TrayField({ label, children, ...rest }: TrayFieldProps) {
  return (
    <div {...rest} className={css.root}>
      <h5>{label}</h5>
      {children}
    </div>
  );
}
