import type { Ref } from 'react';
import type { UIProps } from '../design/index';
import { Icon, Spinner } from '../design';
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
  /** Waiting for the click to take effect: the plus turns into a spinner. */
  busy?: boolean;
};
// A sleeve waits for a badge; a clipboard keeps its dashed placeholder.
export function VacantSlot({ label, onClick, shape, busy, ref, style, ...rest }: VacantSlotProps) {
  return (
    <button
      {...rest}
      ref={ref}
      style={style}
      className={css.root}
      type="button"
      onClick={busy ? undefined : onClick}
      aria-busy={busy || undefined}
      data-shape={shape}
    >
      <span className={peg.peg} data-peg="" />
      {shape === 'badge' && <span className={hang.cord} />}
      {shape === 'badge' && <span className={hang.clip} />}
      <span className={css.outline}>
        {shape === 'badge' && <span className={css.slot} />}
        <span className={css.plus}>
          {busy ? <Spinner size="md" /> : <Icon name="plus" size={20} />}
        </span>
        {label}
      </span>
    </button>
  );
}
