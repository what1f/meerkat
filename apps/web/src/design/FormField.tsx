import type { ReactNode } from 'react';
import type { UIProps } from './types';

import css from './FormField.module.css';
export type FormFieldProps = Omit<UIProps, 'className' | 'style'> & {
  label: string;
  /** Sits at the end of the label line, such as a link to where the value comes from. */
  aside?: ReactNode;
  /** Says what is wrong with the value; the input marks itself invalid. */
  error?: string | null;
  children: ReactNode;
};
// A quiet label above one input in a dialog form.
export function FormField({ label, aside, error, children, ...rest }: FormFieldProps) {
  return (
    <div {...rest} className={css.root}>
      <div className={css.head}>
        <span>{label}</span>
        {aside}
      </div>
      {children}
      {error && (
        <p className={css.error} role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
