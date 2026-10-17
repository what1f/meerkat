import type { UIProps } from './types';

import css from './ThemeThumb.module.css';
export type ThemeThumbProps = Omit<UIProps, 'className' | 'style'> & {
  mode: 'light' | 'dark' | 'system';
  selected: boolean;
  label: string;
  onSelect: () => void;
};
export function ThemeThumb({ mode, selected, label, onSelect, ...rest }: ThemeThumbProps) {
  return (
    <button
      {...rest}
      className={css.root}
      type="button"
      data-mode={mode}
      aria-pressed={selected}
      onClick={onSelect}
    >
      <span aria-hidden="true">
        <span />
        <span />
      </span>
      {label}
    </button>
  );
}
