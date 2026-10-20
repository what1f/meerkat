import type { ReactNode } from 'react';
import type { UIProps } from '../design/index';
import css from './FlipCard.module.css';
export type FlipCardProps = Omit<UIProps, 'className' | 'style'> & {
  flipped: boolean;
  front: ReactNode;
  back: ReactNode;
  onFlip: () => void;
};
// Two faces back to back; a click turns it over. Controls on a face keep their own clicks.
export function FlipCard({ flipped, front, back, onFlip, ...rest }: FlipCardProps) {
  return (
    <div
      {...rest}
      className={css.root}
      data-flipped={flipped}
      onClick={(e) => {
        const hit = (e.target as Element).closest(
          'button, a, input, textarea, select, label, [contenteditable], [role="button"]',
        );
        if (!hit || !e.currentTarget.contains(hit) || hit === e.currentTarget) onFlip();
      }}
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
