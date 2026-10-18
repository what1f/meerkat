import type { Ref } from 'react';
import type { UIProps } from '../design/index';
import { Icon } from '../design';
import type { PegPosition } from './Hang';
import peg from './peg.module.css';
import css from './VacantSlot.module.css';
export type VacantSlotProps = Omit<UIProps, 'className' | 'style'> & {
  ref?: Ref<HTMLButtonElement>;
  /** Where the peg sits on a free board. */
  style?: PegPosition;
  label: string;
  onClick: () => void;
  shape: 'badge' | 'clipboard';
};
// Both shapes keep their peg, so a slot lines up with the badges or the clipboard beside it.
export function VacantSlot({ label, onClick, shape, ref, style, ...rest }: VacantSlotProps) {
  return (
    <button
      {...rest}
      ref={ref}
      style={style}
      className={css.root}
      type="button"
      onClick={onClick}
      data-shape={shape}
    >
      <span className={peg.peg} data-peg="" />
      <span className={css.outline}>
        <span className={css.plus}>
          <Icon name="plus" size={20} />
        </span>
        {label}
      </span>
    </button>
  );
}
