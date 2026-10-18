import type { UIProps } from '../design/index';
import type { Ref } from 'react';
import { Figure } from './Figure';
import type { FigurePack } from './figures';
import css from './RigFigure.module.css';
export type RigFigureProps = Omit<UIProps, 'className' | 'style'> & {
  ref?: Ref<HTMLButtonElement | HTMLSpanElement>;
  id: string;
  pack?: FigurePack;
  seed?: string;
  onClick?: () => void;
  label?: string;
};
export function RigFigure({ ref, id, pack, seed, onClick, label, ...rest }: RigFigureProps) {
  return onClick ? (
    <button
      {...rest}
      ref={ref as Ref<HTMLButtonElement>}
      className={css.root}
      type="button"
      onClick={onClick}
      aria-label={rest['aria-label'] ?? label}
    >
      <Figure id={id} pose="stand" pack={pack} seed={seed} />
    </button>
  ) : (
    <span {...rest} ref={ref as Ref<HTMLSpanElement>} className={css.root}>
      <Figure id={id} pose="stand" pack={pack} seed={seed} />
    </span>
  );
}
