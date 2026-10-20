import type { CSSProperties, ReactNode } from 'react';
import type { UIProps } from './types';

import css from './Segmented.module.css';
export type SegmentedProps<T> = Omit<UIProps, 'className' | 'style'> & {
  options: { value: T; label: string; icon?: ReactNode }[];
  value: T;
  onChange: (value: T) => void;
  size?: 'sm' | 'md';
};
export function Segmented<T>({
  options,
  value,
  onChange,
  size = 'md',
  ...rest
}: SegmentedProps<T>) {
  const selected = Math.max(
    0,
    options.findIndex((option) => Object.is(option.value, value)),
  );
  return (
    <div
      {...rest}
      className={css.root}
      role="radiogroup"
      data-size={size}
      style={{ '--n': Math.max(1, options.length), '--i': selected } as CSSProperties}
    >
      {options.map((option, index) => (
        <button
          key={index}
          type="button"
          role="radio"
          aria-checked={Object.is(option.value, value)}
          tabIndex={index === selected ? 0 : -1}
          onClick={() => onChange(option.value)}
          onKeyDown={(e) => {
            let next = index;
            if (e.key === 'ArrowRight' || e.key === 'ArrowDown')
              next = (index + 1) % options.length;
            else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp')
              next = (index + options.length - 1) % options.length;
            else if (e.key === 'Home') next = 0;
            else if (e.key === 'End') next = options.length - 1;
            else return;
            e.preventDefault();
            const choice = options[next];
            if (choice) {
              onChange(choice.value);
              (e.currentTarget.parentElement?.children[next] as HTMLElement)?.focus();
            }
          }}
        >
          {option.icon}
          {option.label}
        </button>
      ))}
    </div>
  );
}
