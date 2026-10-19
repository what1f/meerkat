import type { ReactNode } from 'react';
import { Icon, Tooltip, type IconName, type UIProps } from '../design';
import css from './Dock.module.css';

export type DockProps = Omit<UIProps, 'className' | 'style'> & {
  team: ReactNode;
  items: { id: string; icon: IconName; label: string; active: boolean; onClick: () => void }[];
};

export function Dock({ team, items, ...rest }: DockProps) {
  return (
    <nav {...rest} className={css.root} data-testid="dock">
      {team}
      <span className={css.separator} aria-hidden="true" />
      {items.map(({ id, icon, label, active, onClick }) => (
        <Tooltip key={id} label={label}>
          {/* eslint-disable-next-line no-restricted-syntax -- Dock navigation has its own design geometry. */}
          <button
            type="button"
            className={css.item}
            data-testid={`nav-${id}`}
            aria-label={label}
            aria-current={active ? 'page' : undefined}
            onClick={onClick}
          >
            <Icon name={icon} size={18} />
          </button>
        </Tooltip>
      ))}
    </nav>
  );
}
