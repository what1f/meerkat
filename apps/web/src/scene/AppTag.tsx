import type { KeyboardEvent, ReactNode } from 'react';
import { useRef } from 'react';
import type { UIProps } from '../design/index';
import { BrandMark } from '../design';
import { swing } from './drag/pointer';
import peg from './peg.module.css';
import css from './AppTag.module.css';
export type AppTagProps = Omit<UIProps, 'className' | 'style'> & {
  brand: 'feishu' | 'telegram';
  label: string;
  selected: boolean;
  onSelect: () => void;
  disabled?: boolean;
  /** Written on the tag under the name while it is picked, e.g. a token line. */
  children?: ReactNode;
};
// A chat app hanging on the board; the team picks one tag, the other fades back.
export function AppTag({
  brand,
  label,
  selected,
  onSelect,
  disabled,
  children,
  ...rest
}: AppTagProps) {
  const own = useRef<HTMLDivElement>(null);
  const pick = () => {
    if (disabled || selected) return;
    onSelect();
    if (own.current) swing(own.current);
  };
  return (
    <div
      {...rest}
      ref={own}
      className={peg.hang}
      role="radio"
      aria-checked={selected}
      aria-disabled={disabled || undefined}
      aria-label={label}
      tabIndex={selected ? 0 : -1}
      data-selected={selected || undefined}
      onClick={pick}
      onKeyDown={(e: KeyboardEvent<HTMLDivElement>) => {
        if (e.target !== e.currentTarget || (e.key !== 'Enter' && e.key !== ' ')) return;
        e.preventDefault();
        pick();
      }}
    >
      <span className={peg.peg} data-peg="" />
      <span className={peg.cord} />
      <div className={peg.shadow}>
        <div className={css.tag}>
          <span className={peg.eyelet} />
          <BrandMark brand={brand} size={56} />
          <span className={css.name}>{label}</span>
          {selected && children && (
            <div className={css.extra} onClick={(e) => e.stopPropagation()}>
              {children}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
