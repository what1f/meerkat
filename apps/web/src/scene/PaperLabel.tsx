import type { UIProps } from '../design/index';
import { InlineText } from '../design';
import css from './PaperLabel.module.css';
export type PaperLabelProps = Omit<UIProps, 'className' | 'style'> & {
  value: string;
  onCommit: (value: string) => void;
  label: string;
};
// The team name: a paper label taped across the top of the board.
export function PaperLabel({ value, onCommit, label, ...rest }: PaperLabelProps) {
  return (
    <div {...rest} className={css.root}>
      <div className={css.strip}>
        <span className={css.tape} />
        <InlineText
          value={value}
          onCommit={onCommit}
          placeholder={label}
          aria-label={label}
          textStyle="heading"
        />
        <span className={css.tape} />
      </div>
    </div>
  );
}
