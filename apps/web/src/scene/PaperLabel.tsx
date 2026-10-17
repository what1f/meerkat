import type { UIProps } from '../design/index';
import { InlineText } from '../design';
import css from './PaperLabel.module.css';
export type PaperLabelProps = Omit<UIProps, 'className' | 'style'> & {
  value: string;
  onCommit: (value: string) => void;
  label: string;
};
export function PaperLabel({ value, onCommit, label, ...rest }: PaperLabelProps) {
  return (
    <div {...rest} className={css.root}>
      <InlineText
        value={value}
        onCommit={onCommit}
        placeholder={label}
        aria-label={label}
        textStyle="heading"
      />
    </div>
  );
}
