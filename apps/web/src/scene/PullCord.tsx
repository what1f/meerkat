import type { UIProps } from '../design/index';

import css from './PullCord.module.css';
export type PullCordProps = Omit<UIProps, 'className' | 'style'> & {
  onToggle: () => void;
  label: string;
};
export function PullCord({ onToggle, label, ...rest }: PullCordProps) {
  return (
    <button {...rest} className={css.root} type="button" onClick={onToggle} aria-label={label}>
      {label}
    </button>
  );
}
