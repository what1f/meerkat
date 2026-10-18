import type { ReactNode } from 'react';
import type { UIProps } from './types';
import css from './StepHeading.module.css';
export type StepHeadingProps = Omit<UIProps, 'className' | 'style'> & {
  children: ReactNode;
  sub?: ReactNode;
};
// A step's serif title, with the quiet line under it.
export function StepHeading({ children, sub, ...rest }: StepHeadingProps) {
  return (
    <>
      <h1 {...rest} className={css.title}>
        {children}
      </h1>
      {sub && <p className={css.sub}>{sub}</p>}
    </>
  );
}
