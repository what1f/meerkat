import type { UIProps } from '../design/index';
import { BrandMark, TextInput } from '../design';
import peg from './peg.module.css';
import css from './TokenTag.module.css';
export type TokenTagProps = Omit<UIProps, 'className' | 'style'> & {
  value: string;
  onChange: (value: string) => void;
  label: string;
  invalid?: boolean;
};
// The bot token is written on the tag, like the rules: a fill-in line, not a box.
export function TokenTag({ value, onChange, label, invalid, ...rest }: TokenTagProps) {
  return (
    <section {...rest} className={peg.hang}>
      <span className={peg.peg} data-peg="" />
      <span className={peg.cord} />
      <div className={peg.shadow}>
        <label className={css.tag}>
          <span className={peg.eyelet} />
          <BrandMark brand="telegram" size={64} />
          <span className={css.label}>{label}</span>
          <TextInput value={value} onChange={onChange} placeholder="" invalid={invalid} />
        </label>
      </div>
    </section>
  );
}
