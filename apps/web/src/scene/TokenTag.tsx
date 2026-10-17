import type { UIProps } from '../design/index';
import { TextInput } from '../design';
import css from './TokenTag.module.css';
export type TokenTagProps = Omit<UIProps, 'className' | 'style'> & {
  value: string;
  onChange: (value: string) => void;
  label: string;
  invalid?: boolean;
};
export function TokenTag({ value, onChange, label, invalid, ...rest }: TokenTagProps) {
  return (
    <label {...rest} className={css.root}>
      {label}
      <TextInput value={value} onChange={onChange} placeholder={label} invalid={invalid} />
    </label>
  );
}
