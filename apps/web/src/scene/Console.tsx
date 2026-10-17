import type { ReactNode } from 'react';
import type { UIProps } from '../design/index';

import css from './Console.module.css';
export type ConsoleProps = Omit<UIProps, 'className' | 'style'> & {
  figure?: ReactNode;
  sticker?: ReactNode;
  notice?: ReactNode;
  screen: ReactNode;
  body?: ReactNode;
  deck: ReactNode;
  drawer?: { label: string; content: ReactNode };
  reading?: boolean;
};
export function Console({
  figure,
  sticker,
  notice,
  screen,
  body,
  deck,
  drawer,
  reading,
  ...rest
}: ConsoleProps) {
  return (
    <section {...rest} className={css.root} data-reading={reading}>
      {figure}
      {sticker}
      {notice}
      {screen}
      {body}
      <div>{deck}</div>
      {drawer && (
        <details>
          <summary>{drawer.label}</summary>
          {drawer.content}
        </details>
      )}
    </section>
  );
}
