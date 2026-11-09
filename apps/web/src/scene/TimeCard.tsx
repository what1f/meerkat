import type { ReactNode, Ref } from 'react';
import type { UIProps } from '../design/index';
import { Icon } from '../design';
import css from './TimeCard.module.css';
export type TimeCardProps = Omit<UIProps, 'className' | 'style'> & {
  ref?: Ref<HTMLButtonElement>;
  /** Who it wakes. */
  avatar: ReactNode;
  /** The day it fires next, or how often it repeats. */
  when: string;
  /** The clock time it fires next. */
  time: string;
  repeats: boolean;
  /** What the member will do when it fires. */
  text: string;
  /** Pulled up out of its pocket while it is being read. */
  lifted?: boolean;
  onClick?: () => void;
};
// A card standing in a rack pocket: the time printed at the top, the job written below.
export function TimeCard({
  avatar,
  when,
  time,
  repeats,
  text,
  lifted,
  ref,
  ...rest
}: TimeCardProps) {
  return (
    <button
      {...rest}
      ref={ref}
      type="button"
      className={css.root}
      data-lifted={lifted || undefined}
    >
      <span className={css.head}>
        <span className={css.when}>
          {repeats && <Icon name="repeat" size={12} />}
          {when}
        </span>
        <span className={css.time}>{time}</span>
        <span className={css.avatar}>{avatar}</span>
      </span>
      <span className={css.text}>{text}</span>
    </button>
  );
}
export type TimeCardNoteProps = Omit<UIProps, 'className' | 'style'> & {
  avatar: ReactNode;
  who: string;
  text: string;
  /** When it fires next, how it repeats, where it reports. */
  facts: { icon: 'clock' | 'repeat' | 'msg'; text: ReactNode }[];
  action: ReactNode;
};
// What a card says in full, shown while it is pulled out.
TimeCard.Note = function Note({ avatar, who, text, facts, action, ...rest }: TimeCardNoteProps) {
  return (
    <div {...rest} className={css.note}>
      <header>
        {avatar}
        <span>{who}</span>
      </header>
      <p>{text}</p>
      <ul>
        {facts.map((fact) => (
          <li key={fact.icon}>
            <Icon name={fact.icon} size={14} />
            {fact.text}
          </li>
        ))}
      </ul>
      <footer>{action}</footer>
    </div>
  );
};
