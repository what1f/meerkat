import {
  useCallback,
  useEffect,
  useRef,
  type KeyboardEvent,
  type ReactNode,
  type Ref,
} from 'react';
import type { UIProps } from '../design/index';
import { swing } from './drag/pointer';
import css from './Hang.module.css';
export type PegPosition = { left?: number; top?: number };
export type HangProps = Omit<UIProps, 'className' | 'style'> & {
  ref?: Ref<HTMLDivElement>;
  /** Where the peg sits on a free board. */
  style?: PegPosition;
  lanyard?: 'member' | 'lead';
  motion?: 'enter' | 'leave' | 'turn-in';
  swingOnHover?: boolean;
  swingKey?: number;
  /** Recedes while something else is being read, e.g. the stage while its personality is open. */
  faded?: boolean;
  /** Greyed out while it needs attention, e.g. a badge whose sign-in has lapsed. */
  muted?: boolean;
  children: ReactNode;
};
// Something hanging from a peg on a lanyard. The peg is the pivot.
export function Hang({
  lanyard = 'member',
  motion,
  swingOnHover,
  swingKey,
  faded,
  muted,
  ref,
  style,
  children,
  onClick,
  onKeyDown,
  ...rest
}: HangProps) {
  const own = useRef<HTMLDivElement>(null);
  const attach = useCallback(
    (el: HTMLDivElement | null) => {
      own.current = el;
      if (typeof ref === 'function') return ref(el);
      if (ref) ref.current = el;
    },
    [ref],
  );
  // Every change of swingKey swings once, 0 included; mounting does not.
  const seen = useRef(swingKey);
  useEffect(() => {
    if (swingKey === undefined || swingKey === seen.current) return;
    seen.current = swingKey;
    if (own.current) swing(own.current);
  }, [swingKey]);
  return (
    <div
      {...rest}
      // With onClick it is a card you open: focusable, and Enter or Space opens it too.
      {...(onClick && {
        role: 'button',
        tabIndex: 0,
        onClick,
        onKeyDown: (e: KeyboardEvent<HTMLDivElement>) => {
          onKeyDown?.(e);
          if (e.defaultPrevented || e.target !== e.currentTarget) return;
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            e.currentTarget.click();
          }
        },
      })}
      {...(!onClick && { onKeyDown })}
      ref={attach}
      style={style}
      className={css.root}
      data-lanyard={lanyard}
      data-motion={motion}
      data-swing-hover={swingOnHover}
      data-faded={faded}
      data-muted={muted}
    >
      <span className={css.peg} data-peg="" />
      <span className={css.cord} />
      <span className={css.clip} />
      {children}
    </div>
  );
}
