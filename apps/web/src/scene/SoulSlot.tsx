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
// Personality: a card standing in a slot. Focus pulls it out to read.
export function SoulSlot({
  value,
  onChange,
  label,
  placeholder,
  onFocusChange,
  ...rest
}: SoulSlotProps) {
  return (
    <label {...rest} className={css.root} data-long={value.length > 60}>
      <span className={css.engrave}>{label}</span>
      <span className={css.well}>
        <span className={css.slip}>
          <DocArea
            value={value}
            onChange={onChange}
            placeholder={placeholder}
            minRows={1}
            onFocus={() => onFocusChange(true)}
            onBlur={() => onFocusChange(false)}
          />
        </span>
        <span className={css.lip} />
      </span>
    </label>
  );
}
