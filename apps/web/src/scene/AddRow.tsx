import type { UIProps } from '../design/index';

import css from './AddRow.module.css';
export type AddRowProps = Omit<UIProps, 'className' | 'style'> & {
  label: string;
  onClick: () => void;
};
export function AddRow({ label, onClick, ...rest }: AddRowProps) {
  return (
    <button {...rest} className={css.root} type="button" onClick={onClick}>
      {label}
    </button>
  );
}
