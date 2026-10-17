import type { ReactNode } from 'react';
import type { UIProps } from './types';

import css from './OptionColumn.module.css';
export type OptionColumnProps = Omit<UIProps, 'className' | 'style'> & {
  title: string;
  children: ReactNode;
};
export function OptionColumn({ title, children, ...rest }: OptionColumnProps) {
  return (
    <section {...rest} className={css.root}>
      <h3>{title}</h3>
      <div role="listbox" aria-label={title}>
        {children}
      </div>
    </section>
  );
}
export type OptionColumnItemProps = Omit<UIProps, 'className' | 'style'> & {
  selected?: boolean;
  dimmed?: boolean;
  mono?: boolean;
  leading?: ReactNode;
  onSelect: () => void;
  children: ReactNode;
};
OptionColumn.Item = function Item({
  selected = false,
  dimmed,
  mono,
  leading,
  onSelect,
  children,
  ...rest
}: OptionColumnItemProps) {
  return (
    <button
      {...rest}
      className={css.root}
      type="button"
      role="option"
      aria-selected={selected}
      data-dimmed={dimmed}
      data-mono={mono}
      onClick={onSelect}
      onKeyDown={(e) => {
        const items = Array.from(
          e.currentTarget
            .closest('[role="listbox"]')
            ?.querySelectorAll<HTMLButtonElement>('[role="option"]') ?? [],
        );
        const index = items.indexOf(e.currentTarget);
        const next =
          e.key === 'ArrowDown'
            ? (index + 1) % items.length
            : e.key === 'ArrowUp'
              ? (index + items.length - 1) % items.length
              : e.key === 'Home'
                ? 0
                : e.key === 'End'
                  ? items.length - 1
                  : undefined;
        if (next !== undefined) {
          e.preventDefault();
          items[next]?.focus();
        }
      }}
    >
      {leading}
      {children}
    </button>
  );
};
