import type { Ref } from 'react';
import type { UIProps } from './types';
import { Icon } from './Icon';
import type { IconName } from './icons';
import css from './IconButton.module.css';
export type IconButtonProps = Omit<UIProps, 'className' | 'style'> & {
  ref?: Ref<HTMLButtonElement>;
  icon: IconName;
  label: string;
  size?: 'sm' | 'md';
  onClick: () => void;
};
export function IconButton({ icon, label, size = 'md', onClick, ...rest }: IconButtonProps) {
  return (
    <button
      {...rest}
      className={css.root}
      type="button"
      data-size={size}
      onClick={onClick}
      title={label}
      aria-label={label}
    >
      <Icon name={icon} />
    </button>
  );
}
