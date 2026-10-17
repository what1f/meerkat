import type { UIProps } from './types';

import css from './Skeleton.module.css';
export type SkeletonProps = Omit<UIProps, 'className' | 'style'> & {
  shape: 'line' | 'circle';
  width: number | string;
  height?: number | string;
};
export function Skeleton({ shape, width, height, ...rest }: SkeletonProps) {
  return (
    <span
      {...rest}
      className={css.root}
      data-shape={shape}
      aria-hidden="true"
      style={{ width, height }}
    />
  );
}
