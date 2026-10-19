import type { ReactNode } from 'react';
import type { UIProps } from '../design/index';
import { Icon } from '../design';
import css from './BadgeVerso.module.css';
export type BadgeVersoProps = Omit<UIProps, 'className' | 'style'> & {
  mark: ReactNode;
  actions: { label: string; onClick: () => void; busy?: boolean }[];
};
// The back of your own badge: the chat account, with tear-off actions.
export function BadgeVerso({ mark, actions, ...rest }: BadgeVersoProps) {
  return (
    <div {...rest} className={css.root}>
      <span className={css.slot} />
      <span className={css.org}>{mark}</span>
      <span className={css.tear}>
        {actions.map((a, i) => (
          <button key={i} type="button" aria-busy={a.busy} onClick={a.onClick}>
            {a.busy && <Icon name="refresh" size={14} />}
            {a.label}
          </button>
        ))}
      </span>
    </div>
  );
}
