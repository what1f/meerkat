import { useState, type ReactNode } from 'react';
import type { UIProps } from '../design/index';
import { Icon, Reveal } from '../design';
import peg from './peg.module.css';
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
// A console mounted on the pegboard. The drawer is a tray pulled out from under it.
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
  const [open, setOpen] = useState(false);
  return (
    <section {...rest} className={css.root} data-reading={reading}>
      {notice && <div className={css.notice}>{notice}</div>}
      <span className={css.hook}>
        <span className={peg.peg} data-peg="" />
      </span>
      <span className={css.hook}>
        <span className={peg.peg} />
      </span>
      {figure && <div className={css.figure}>{figure}</div>}
      <div className={css.body}>
        {sticker && <div className={css.sticker}>{sticker}</div>}
        {screen}
        {body}
        <div className={css.deck}>{deck}</div>
        {drawer && (
          <button
            className={css.tab}
            type="button"
            aria-expanded={open}
            onClick={() => setOpen(!open)}
          >
            {drawer.label}
            <Icon name="chev" size={12} />
          </button>
        )}
      </div>
      {drawer && (
        <div className={css.drawer}>
          <Reveal open={open}>
            <div className={css.tray}>{drawer.content}</div>
          </Reveal>
        </div>
      )}
    </section>
  );
}
