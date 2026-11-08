import type { ReactNode } from 'react';
import type { UIProps } from './types';
import { Select as BaseSelect } from '@base-ui/react/select';
import { Icon } from './Icon';
import { useLayer } from './layer';
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
  const layer = useLayer();
  return (
    <BaseSelect.Root
      value={value}
      onValueChange={(next) => {
        if (next !== null) onChange(next);
      }}
      items={options}
    >
      <BaseSelect.Trigger {...rest} className={css.root} data-variant={variant}>
        <BaseSelect.Value>
          {(current: T) => {
            const option = options.find((o) => Object.is(o.value, current));
            return (
              <>
                {option?.leading}
                <span>{option?.label}</span>
              </>
            );
          }}
        </BaseSelect.Value>
        <BaseSelect.Icon className={css.chev}>
          <Icon name="chev" size={14} />
        </BaseSelect.Icon>
      </BaseSelect.Trigger>
      <BaseSelect.Portal container={layer.container}>
        <BaseSelect.Positioner
          positionMethod={layer.positionMethod}
          className={css.positioner}
          alignItemWithTrigger={false}
          sideOffset={4}
          align="start"
        >
          <BaseSelect.Popup className={css.popup}>
            {options.map((option, index) => (
              <BaseSelect.Item
                key={index}
                className={css.item}
                value={option.value}
                disabled={option.disabled}
              >
                {option.leading}
                <BaseSelect.ItemText className={css.label}>{option.label}</BaseSelect.ItemText>
                <BaseSelect.ItemIndicator className={css.check}>
                  <Icon name="check" size={14} />
                </BaseSelect.ItemIndicator>
              </BaseSelect.Item>
            ))}
          </BaseSelect.Popup>
        </BaseSelect.Positioner>
      </BaseSelect.Portal>
    </BaseSelect.Root>
  );
}
