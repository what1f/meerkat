import type { ReactNode } from 'react';
import type { UIProps } from '../design/index';
import css from './FlipCard.module.css';
export type FlipCardProps = Omit<UIProps, 'className' | 'style'> & {
  flipped: boolean;
  front: ReactNode;
  back: ReactNode;
  onFlip: () => void;
};
// Two faces back to back; double-click turns it over.
export function FlipCard({ flipped, front, back, onFlip, ...rest }: FlipCardProps) {
  return (
    <div
      {...rest}
      className={css.root}
      data-flipped={flipped}
      onDoubleClick={onFlip}
      onKeyDown={(e) => {
        if (e.target === e.currentTarget && (e.key === 'Enter' || e.key === ' ')) {
          e.preventDefault();
          onFlip();
        }
      }}
      tabIndex={0}
    >
      <div className={css.face} inert={flipped}>
        {front}
      </div>
      <div className={css.face} inert={!flipped}>
        {back}
      </div>
    </div>
  );
}
