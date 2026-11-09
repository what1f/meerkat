import type { ReactNode, Ref } from 'react';
import type { UIProps } from '../design/index';
import type { PegPosition } from './Hang';
import peg from './peg.module.css';
import css from './TimeCardRack.module.css';
export type TimeCardRackProps = Omit<UIProps, 'className' | 'style'> & {
  ref?: Ref<HTMLElement>;
  /** Where the peg sits on a free board. */
  style?: PegPosition;
  /** Engraved on the plate at the top. */
  label: string;
  /** One `TimeCard` per pocket; the rack always shows a few pockets, empty ones included. */
  children: ReactNode[];
};
const least = 3;
// A steel time-card rack on a peg: an engraved plate, then one pocket per card.
export function TimeCardRack({ label, children, ref, style, ...rest }: TimeCardRackProps) {
  const empty = Math.max(0, least - children.length);
  return (
    <section {...rest} ref={ref} style={style} className={css.root}>
      <span className={peg.peg} data-peg="" />
      <div className={css.rack}>
        <span className={css.hole} />
        <span className={css.plate}>{label}</span>
        <ul className={css.pockets}>
          {children.map((card, index) => (
            <li key={index} className={css.pocket}>
              {card}
            </li>
          ))}
          {Array.from({ length: empty }, (_, index) => (
            <li key={`empty-${index}`} className={css.pocket} data-empty="" />
          ))}
        </ul>
      </div>
    </section>
  );
}
