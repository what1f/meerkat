import { Avatar } from '../design';
import type { ReactNode } from 'react';
import type { UIProps, Status } from '../design/index';

import css from './Badge.module.css';
export type BadgeProps = Omit<UIProps, 'className' | 'style'> & {
  avatar: {
    src?: string;
    status?: Status;
    statusTip?: string;
    creating?: boolean;
    onEdit?: () => void;
    editLabel?: string;
  };
  name: ReactNode;
  role?: ReactNode;
  bio?: ReactNode;
  foot?: ReactNode;
  size: 'card' | 'stage' | 'intro';
  dimmed?: boolean;
  ghost?: boolean;
};
export function Badge({ avatar, name, role, bio, foot, size, dimmed, ghost, ...rest }: BadgeProps) {
  return (
    <article {...rest} className={css.root} data-size={size} data-dimmed={dimmed} aria-busy={ghost}>
      <Avatar {...avatar} size="fill" dimmed={dimmed} />
      <header>{name}</header>
      <div>{role}</div>
      <div>{bio}</div>
      <footer>{foot}</footer>
    </article>
  );
}
