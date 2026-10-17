import type { UIProps } from '../design/index';
import { figureFrame, type Pose } from './figures';
import css from './Figure.module.css';
export type FigureProps = Omit<UIProps, 'className' | 'style'> & {
  id: string;
  pose: Pose;
  scale?: number;
};
export function Figure({ id, pose, scale = 1, ...rest }: FigureProps) {
  const frame = figureFrame(id, pose, scale);
  if (!frame) return null;
  return <img {...rest} className={css.root} {...frame} alt="" />;
}
