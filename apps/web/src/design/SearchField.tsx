import type { UIProps } from './types';
import { Icon } from './Icon';
import css from './SearchField.module.css';
export type SearchFieldProps = Omit<UIProps, 'className' | 'style'> & {
  value: string;
  onChange: (value: string) => void;
  placeholder: string;
  autoFocus?: boolean;
  onEnter?: (value: string) => void;
};
export function SearchField({
  value,
  onChange,
  placeholder,
  autoFocus,
  onEnter,
  ...rest
}: SearchFieldProps) {
  return (
    <label className={css.root}>
      <Icon name="search" />
      <input
        {...rest}
        type="search"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        aria-label={rest['aria-label'] ?? placeholder}
        autoFocus={autoFocus}
        onKeyDown={(e) => {
          rest.onKeyDown?.(e);
          if (!e.defaultPrevented && e.key === 'Enter' && !e.nativeEvent.isComposing) {
            e.preventDefault();
            onEnter?.(value);
          }
        }}
      />
    </label>
  );
}
