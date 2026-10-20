import type { ReactNode } from 'react';
import type { UIProps } from '../design/index';
import type { Ref } from 'react';
import css from './ConsoleScreen.module.css';
export type ConsoleScreenProps = Omit<UIProps, 'className' | 'style'> & {
  ref?: Ref<HTMLButtonElement | HTMLDivElement>;
  onClick?: () => void;
  variant: 'readout' | 'list';
  children: ReactNode;
};
export function ConsoleScreen({ ref, onClick, variant, children, ...rest }: ConsoleScreenProps) {
  return onClick ? (
    <button
      {...rest}
      ref={ref as Ref<HTMLButtonElement>}
      className={css.root}
      type="button"
      data-variant={variant}
      onClick={onClick}
    >
      {children}
    </button>
  ) : (
    <div {...rest} ref={ref as Ref<HTMLDivElement>} className={css.root} data-variant={variant}>
      {children}
    </div>
  );
}
