import type { Ref } from 'react';
import type { ReactNode } from 'react';
import type { UIProps } from './types';
import { Icon } from './Icon';
import css from './LinkButton.module.css';
export type LinkButtonProps = Omit<UIProps, 'className' | 'style'> & {
  ref?: Ref<HTMLButtonElement>;
  children: ReactNode;
  disclosure?: boolean;
  /** Opens another app or site: underlined, with an outgoing arrow. */
  external?: boolean;
  open?: boolean;
  /** Optional only when the button is a Menu or Popover trigger. */
  onClick?: () => void;
};
export function LinkButton({
  children,
  disclosure,
  external,
  open,
  onClick,
  ...rest
}: LinkButtonProps) {
  return (
    <button
      {...rest}
      className={css.root}
      type="button"
      data-external={external}
      onClick={onClick}
      aria-expanded={rest['aria-expanded'] ?? (disclosure ? open : undefined)}
    >
      {children}
      {external && <Icon name="out" size={12} />}
      {disclosure && <Icon name="chev" size={12} />}
    </button>
  );
}
