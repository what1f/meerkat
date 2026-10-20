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
      <span className={css.window} aria-hidden="true">
        {(mode === 'system' ? (['light', 'dark'] as const) : [mode]).map((tone) => (
          <span key={tone} className={css.mini} data-tone={tone}>
            <span>
              <i />
              <i />
              <i />
            </span>
            <span />
          </span>
        ))}
      </span>
      {label}
    </button>
  );
}
