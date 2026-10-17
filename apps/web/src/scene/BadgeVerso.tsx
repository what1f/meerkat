import type { ReactNode } from 'react';
import type { UIProps } from '../design/index';
import { Button, Spinner } from '../design';
import css from './BadgeVerso.module.css';
export type BadgeVersoProps = Omit<UIProps, 'className' | 'style'> & {
  mark: ReactNode;
  actions: { label: string; onClick: () => void; busy?: boolean }[];
};
export function BadgeVerso({ mark, actions, ...rest }: BadgeVersoProps) {
  return (
    <div {...rest} className={css.root}>
      {mark}
      {actions.map((a, i) => (
        <Button
          key={i}
          loading={a.busy}
          leading={a.busy ? <Spinner /> : undefined}
          onClick={a.onClick}
        >
          {a.label}
        </Button>
      ))}
    </div>
  );
}
