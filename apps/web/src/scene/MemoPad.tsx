import type { Ref } from 'react';
import type { UIProps } from '../design/index';
import type { PegPosition } from './Hang';
import peg from './peg.module.css';
import css from './MemoPad.module.css';
const lines = 4;
function Coil({ turns }: { turns: number }) {
  return (
    <span className={css.coil} aria-hidden="true">
      {Array.from({ length: turns }, (_, i) => (
        <i key={i} />
      ))}
    </span>
  );
}
export type MemoPadProps = Omit<UIProps, 'className' | 'style'> & {
  ref?: Ref<HTMLButtonElement>;
  /** Where the peg sits on a free board. */
  style?: PegPosition;
  title: string;
  /** The first few open items, written on the top sheet. */
  items: string[];
  /** The list could not be read. */
  alert?: boolean;
  onClick: () => void;
};
// A spiral memo pad on a kraft back, hung on a peg. A click takes it off the wall.
export function MemoPad({ title, items, alert, ref, style, ...rest }: MemoPadProps) {
  const shown = items.slice(0, lines);
  return (
    <button
      {...rest}
      ref={ref}
      style={style}
      type="button"
      className={css.root}
      aria-haspopup="dialog"
    >
      <span className={peg.peg} data-peg="" />
      <span className={css.back}>
        <Coil turns={12} />
        <span className={css.paper}>
          <span className={css.title}>
            <span>{title}</span>
            {alert && <span className={css.alert} />}
          </span>
          {Array.from({ length: lines }, (_, i) => (
            <span key={i} className={css.line} data-empty={i >= shown.length || undefined}>
              {i < shown.length && (
                <>
                  <span className={css.box} />
                  <span className={css.text}>{shown[i]}</span>
                </>
              )}
            </span>
          ))}
        </span>
      </span>
    </button>
  );
}
