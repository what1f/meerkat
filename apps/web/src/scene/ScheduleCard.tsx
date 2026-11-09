import type { ReactNode, Ref } from 'react';
import type { UIProps } from '../design/index';
import type { PegPosition } from './Hang';
import peg from './peg.module.css';
import css from './ScheduleCard.module.css';
const slots = 4;
function Rings({ count }: { count: number }) {
  return (
    <span className={css.rings} aria-hidden="true">
      {Array.from({ length: count }, (_, i) => (
        <i key={i} />
      ))}
    </span>
  );
}
export type ScheduleEntry = { day: string; time: string; avatar: ReactNode };
export type ScheduleCardProps = Omit<UIProps, 'className' | 'style'> & {
  ref?: Ref<HTMLButtonElement>;
  /** Where the peg sits on a free board. */
  style?: PegPosition;
  title: string;
  /** What comes next, soonest first. */
  entries: ScheduleEntry[];
  /** The schedule could not be read. */
  alert?: boolean;
  onClick: () => void;
};
// A ring-bound wall schedule: a colored band on the peg, the next few times on the page. A
// click opens the full schedule.
export function ScheduleCard({ title, entries, alert, ref, style, ...rest }: ScheduleCardProps) {
  const shown = entries.slice(0, slots);
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
      <span className={css.card}>
        <span className={css.band}>
          {title}
          {alert && <span className={css.alert} />}
        </span>
        <Rings count={2} />
        <span className={css.page}>
          {Array.from({ length: slots }, (_, i) => {
            const entry = shown[i];
            const first = entry && shown[i - 1]?.day !== entry.day;
            return (
              <span key={i} className={css.slot}>
                {entry && (
                  <>
                    <span className={css.day}>{first ? entry.day : ''}</span>
                    <span className={css.time}>{entry.time}</span>
                    <span className={css.who}>{entry.avatar}</span>
                  </>
                )}
              </span>
            );
          })}
        </span>
      </span>
    </button>
  );
}
