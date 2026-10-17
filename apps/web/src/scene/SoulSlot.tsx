import type { UIProps } from '../design/index';
import { DocArea } from '../design';
import css from './SoulSlot.module.css';
export type SoulSlotProps = Omit<UIProps, 'className' | 'style'> & {
  value: string;
  onChange: (value: string) => void;
  label: string;
  placeholder: string;
  onFocusChange: (reading: boolean) => void;
};
export function SoulSlot({
  value,
  onChange,
  label,
  placeholder,
  onFocusChange,
  ...rest
}: SoulSlotProps) {
  return (
    <label {...rest} className={css.root}>
      {label}
      <DocArea
        value={value}
        onChange={onChange}
        placeholder={placeholder}
        onFocus={() => onFocusChange(true)}
        onBlur={() => onFocusChange(false)}
      />
    </label>
  );
}
