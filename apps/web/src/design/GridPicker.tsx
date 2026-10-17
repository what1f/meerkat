import type { ReactNode } from 'react';
import type { UIProps } from './types';
import { useState } from 'react';
import css from './GridPicker.module.css';
export type GridPickerProps<T> = Omit<UIProps, 'className' | 'style'> & {
  cells: { value: T; label?: string; content: ReactNode }[];
  value?: T;
  onPick: (value: T) => void;
  leading?: ReactNode;
  trailing?: ReactNode;
  columns: number;
};
export function GridPicker<T>({
  cells,
  value,
  onPick,
  leading,
  trailing,
  columns,
  ...rest
}: GridPickerProps<T>) {
  const [focused, setFocused] = useState<{ value: T } | null>(null);
  const remembered = focused ? cells.findIndex((c) => Object.is(c.value, focused.value)) : -1;
  const selected = cells.findIndex((c) => Object.is(c.value, value));
  const focus = remembered >= 0 ? remembered : Math.max(0, selected);
  const count = Math.max(1, Math.floor(columns));
  return (
    <div
      {...rest}
      className={css.root}
      role="grid"
      style={{ gridTemplateColumns: `repeat(${count}, 1fr)` }}
    >
      {leading && (
        <div role="row">
          <div role="gridcell">{leading}</div>
        </div>
      )}
      {Array.from({ length: Math.ceil(cells.length / count) }, (_, row) => (
        <div role="row" key={row}>
          {cells.slice(row * count, (row + 1) * count).map((cell, offset) => {
            const index = row * count + offset;
            return (
              <div role="gridcell" key={index} aria-selected={Object.is(cell.value, value)}>
                <button
                  type="button"
                  tabIndex={focus === index ? 0 : -1}
                  aria-label={cell.label}
                  data-cell-index={index}
                  onFocus={() => setFocused({ value: cell.value })}
                  onClick={() => onPick(cell.value)}
                  onKeyDown={(e) => {
                    const step = {
                      ArrowRight: 1,
                      ArrowLeft: -1,
                      ArrowDown: count,
                      ArrowUp: -count,
                    }[e.key];
                    if (step === undefined && e.key !== 'Home' && e.key !== 'End') return;
                    e.preventDefault();
                    const next =
                      e.key === 'Home'
                        ? 0
                        : e.key === 'End'
                          ? cells.length - 1
                          : Math.max(0, Math.min(cells.length - 1, index + (step ?? 0)));
                    const nextCell = cells[next];
                    if (nextCell) setFocused({ value: nextCell.value });
                    e.currentTarget
                      .closest('[role="grid"]')
                      ?.querySelector<HTMLButtonElement>(`button[data-cell-index="${next}"]`)
                      ?.focus();
                  }}
                >
                  {cell.content}
                </button>
              </div>
            );
          })}
        </div>
      ))}
      {trailing && (
        <div role="row">
          <div role="gridcell">{trailing}</div>
        </div>
      )}
    </div>
  );
}
