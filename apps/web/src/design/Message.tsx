import { Icon } from './Icon';
import type { CSSProperties, ReactNode } from 'react';
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
      style={delay ? ({ '--delay': `${delay}ms` } as CSSProperties) : undefined}
      aria-busy={typing}
    >
      {avatar && <img src={avatar} alt="" />}
      <div>
        {variant === 'done' && (
          <span className={css.tick}>
            <Icon name="check" size={10} />
          </span>
        )}
        {typing ? (
          <span className={css.dots}>
            <i />
            <i />
            <i />
          </span>
        ) : (
          children
        )}
      </div>
    </div>
  );
}
