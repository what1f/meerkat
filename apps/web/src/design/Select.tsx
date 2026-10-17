import type { ReactNode } from 'react';
import type { UIProps } from './types';
import { Select as BaseSelect } from '@base-ui/react/select';
import css from './Select.module.css';
export type SelectProps<T> = Omit<UIProps, 'className' | 'style'> & {
  options: { value: T; label: string; leading?: ReactNode; disabled?: boolean }[];
  value: T;
  onChange: (value: T) => void;
  variant?: 'quiet' | 'boxed';
};
export function Select<T>({
  options,
  value,
  onChange,
  variant = 'quiet',
  ...rest
}: SelectProps<T>) {
  return (
    <BaseSelect.Root
      value={value}
      onValueChange={(next) => {
        if (next !== null) onChange(next);
      }}
      items={options}
    >
      <BaseSelect.Trigger {...rest} className={css.root} data-variant={variant}>
        <BaseSelect.Value />
      </BaseSelect.Trigger>
      <BaseSelect.Portal>
        <BaseSelect.Positioner>
          <BaseSelect.Popup>
            {options.map((option, index) => (
              <BaseSelect.Item key={index} value={option.value} disabled={option.disabled}>
                {option.leading}
                <BaseSelect.ItemText>{option.label}</BaseSelect.ItemText>
              </BaseSelect.Item>
            ))}
          </BaseSelect.Popup>
        </BaseSelect.Positioner>
      </BaseSelect.Portal>
    </BaseSelect.Root>
  );
}
