import type { UIProps } from '../design/index';
import type { Ref } from 'react';
import { Icon } from '../design';
import { Reaction } from './Reaction';
import css from './Sticker.module.css';
export type StickerProps = Omit<UIProps, 'className' | 'style'> & {
  ref?: Ref<HTMLButtonElement>;
  reaction: string | null;
  onClick: () => void;
  label: string;
};
export function Sticker({ reaction, onClick, label, ...rest }: StickerProps) {
  return (
    <button {...rest} className={css.root} type="button" onClick={onClick} aria-label={label}>
      {reaction ? <Reaction id={reaction} /> : <Icon name="smile" />}
    </button>
  );
}
