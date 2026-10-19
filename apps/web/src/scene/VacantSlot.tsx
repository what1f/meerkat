import type { Ref } from 'react';
import type { UIProps } from '../design/index';
import { Icon } from '../design';
import type { PegPosition } from './Hang';
import peg from './peg.module.css';
import css from './VacantSlot.module.css';
import hang from './Hang.module.css';
export type VacantSlotProps = Omit<UIProps, 'className' | 'style'> & {
  ref?: Ref<HTMLButtonElement>;
  /** Where the peg sits on a free board. */
  style?: PegPosition;
  label: string;
  onClick: () => void;
  shape: 'badge' | 'clipboard';
};
// A sleeve waits for a badge; a clipboard keeps its dashed placeholder.
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
      {shape === 'badge' && <span className={hang.cord} />}
      {shape === 'badge' && <span className={hang.clip} />}
      <span className={css.outline}>
        {shape === 'badge' && <span className={css.slot} />}
        <span className={css.plus}>
          <Icon name="plus" size={20} />
        </span>
        {label}
      </span>
    </button>
  );
}
