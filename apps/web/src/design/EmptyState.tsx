import type { ReactNode } from 'react';
import type { UIProps } from './types';
import mascot from '../assets/mascot/meerkat-peek.png';
import css from './EmptyState.module.css';
export type EmptyStateProps = Omit<UIProps, 'className' | 'style'> & {
  title: string;
  action?: ReactNode;
};
export function EmptyState({ title, action, ...rest }: EmptyStateProps) {
  return (
    <section {...rest} className={css.root}>
      <div className={css.peek}>
        <img src={mascot} alt="" />
      </div>
      <h2>{title}</h2>
      {action}
    </section>
  );
}
