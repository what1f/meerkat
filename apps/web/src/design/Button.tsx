import type { Ref } from 'react';
import type { ReactNode } from 'react';
import type { UIProps } from './types';

import css from './Button.module.css';
export type ButtonProps = Omit<UIProps<HTMLButtonElement>, 'className' | 'style'> & {
  ref?: Ref<HTMLButtonElement>;
  variant?: 'default' | 'primary' | 'quiet' | 'danger';
  size?: 'sm' | 'md' | 'lg';
  disabled?: boolean;
  loading?: boolean;
  leading?: ReactNode;
  children: ReactNode;
};
export function Button({
  variant = 'default',
  size = 'md',
  disabled,
  loading,
  leading,
  children,
  ...rest
}: ButtonProps) {
  return (
    <button
      {...rest}
      className={css.root}
      type="button"
      data-variant={variant}
      data-size={size}
      disabled={disabled || loading}
      aria-busy={loading}
    >
      {leading}
      {children}
    </button>
  );
}
