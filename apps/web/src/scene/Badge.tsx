import { Avatar } from '../design';
import type { ReactNode, Ref } from 'react';
import type { PegPosition } from './Hang';
import type { UIProps, Status } from '../design/index';
import css from './Badge.module.css';
export type BadgeProps = Omit<UIProps, 'className' | 'style'> & {
  ref?: Ref<HTMLElement>;
  style?: PegPosition;
  avatar: {
    src?: string;
    fallback?: string;
    status?: Status;
    statusTip?: string;
    creating?: boolean;
    onEdit?: () => void;
    editLabel?: string;
  };
  name: ReactNode;
  role?: ReactNode;
  roleText?: string;
  bio?: ReactNode;
  foot?: ReactNode;
  size: 'card' | 'stage' | 'intro';
  dimmed?: boolean;
  ghost?: boolean;
};
// Member badge: a plain white card. The avatar carries the color.
export function Badge({
  ref,
  style,
  avatar,
  name,
  role,
  roleText,
  bio,
  foot,
  size,
  dimmed,
  ghost,
  ...rest
}: BadgeProps) {
  return (
    <article
      {...rest}
      ref={ref}
      style={style}
      className={css.root}
      data-size={size}
      data-dimmed={dimmed}
      aria-busy={ghost}
    >
      <span className={css.slot} />
      <div className={css.avatar}>
        <Avatar {...avatar} size="fill" dimmed={dimmed} />
      </div>
      <header className={css.name}>{name}</header>
      <div
        className={css.role}
        data-latin={/^[\x00-\x7f]+$/.test(typeof role === 'string' ? role : (roleText ?? ''))}
      >
        {role}
      </div>
      {bio != null && (
        <div className={css.bio} data-testid="badge-bio">
          {bio}
        </div>
      )}
      {foot !== undefined && <footer className={css.foot}>{foot}</footer>}
    </article>
  );
}
