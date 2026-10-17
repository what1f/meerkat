import { Icon } from './Icon';
import type { ReactNode } from 'react';
import type { UIProps } from './types';

import css from './Message.module.css';
export type MessageProps = Omit<UIProps, 'className' | 'style'> & {
  from: 'me' | 'them';
  variant?: 'bubble' | 'done';
  avatar?: string;
  typing?: boolean;
  delay?: number;
  animate?: boolean;
  children: ReactNode;
};
export function Message({
  from,
  variant = 'bubble',
  avatar,
  typing,
  delay,
  animate = true,
  children,
  ...rest
}: MessageProps) {
  return (
    <div
      {...rest}
      className={css.root}
      data-from={from}
      data-variant={variant}
      data-animate={animate}
      data-delay={delay}
      aria-busy={typing}
    >
      {avatar && <img src={avatar} alt="" />}
      <div>
        {variant === 'done' && <Icon name="check" />}
        {children}
      </div>
    </div>
  );
}
