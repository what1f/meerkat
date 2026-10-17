import type { UIProps, Status } from './types';

import css from './StatusDot.module.css';
export type StatusDotProps = Omit<UIProps, 'className' | 'style'> & {
  status: Status;
  tip?: string;
};
export function StatusDot({ status, tip, ...rest }: StatusDotProps) {
  return (
    <span
      {...rest}
      className={css.root}
      data-status={status}
      role="status"
      title={tip}
      aria-label={tip}
    />
  );
}
