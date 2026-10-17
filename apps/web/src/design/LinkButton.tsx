import type { Ref } from 'react';
import type { ReactNode } from 'react';
import type { UIProps } from './types';
import { Icon } from './Icon';
import css from './LinkButton.module.css';
export type LinkButtonProps = Omit<UIProps, 'className' | 'style'> & {
  ref?: Ref<HTMLButtonElement>;
  children: ReactNode;
  disclosure?: boolean;
  open?: boolean;
  onClick: () => void;
};
export function LinkButton({ children, disclosure, open, onClick, ...rest }: LinkButtonProps) {
  return (
    <button
      {...rest}
      className={css.root}
      type="button"
      onClick={onClick}
      aria-expanded={rest['aria-expanded'] ?? (disclosure ? open : undefined)}
    >
      {children}
      {disclosure && <Icon name={open ? 'up' : 'chev'} />}
    </button>
  );
}
