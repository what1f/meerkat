import type { UIProps } from './types';

import css from './TextInput.module.css';
export type TextInputProps = Omit<UIProps, 'className' | 'style'> & {
  value: string;
  onChange: (value: string) => void;
  size?: 'sm' | 'md' | 'lg';
  align?: 'start' | 'center';
  mono?: boolean;
  invalid?: boolean;
  placeholder: string;
};
export function TextInput({
  value,
  onChange,
  size = 'md',
  align = 'start',
  mono,
  invalid,
  placeholder,
  ...rest
}: TextInputProps) {
  return (
    <input
      {...rest}
      className={css.root}
      value={value}
      onChange={(e) => onChange(e.target.value)}
      data-size={size}
      data-align={align}
      data-mono={mono}
      aria-invalid={invalid}
      placeholder={placeholder}
    />
  );
}
