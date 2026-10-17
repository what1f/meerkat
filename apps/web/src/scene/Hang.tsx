import type { ReactNode } from 'react';
import type { UIProps } from '../design/index';

import css from './Hang.module.css';
export type HangProps = Omit<UIProps, 'className' | 'style'> & {
  lanyard?: 'member' | 'lead';
  motion?: 'enter' | 'leave' | 'turn-in';
  swingOnHover?: boolean;
  swingKey?: number;
  children: ReactNode;
};
export function Hang({
  lanyard = 'member',
  motion,
  swingOnHover,
  swingKey,
  children,
  ...rest
}: HangProps) {
  return (
    <div
      {...rest}
      className={css.root}
      data-lanyard={lanyard}
      data-motion={motion}
      data-swing={swingOnHover}
      data-swing-key={swingKey}
    >
      {children}
    </div>
  );
}
