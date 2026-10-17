import type { UIProps } from '../design/index';
import { figures } from './figures';
const frames = import.meta.glob<string>('../assets/figures/*/*.png', {
  eager: true,
  query: '?url',
  import: 'default',
});
import css from './Figure.module.css';
export type FigureProps = Omit<UIProps, 'className' | 'style'> & {
  id: string;
  pose: 'work' | 'sleep' | 'carry' | 'stand';
  scale?: number;
};
export function Figure({ id, pose, scale = 1, ...rest }: FigureProps) {
  const frame = figures[id]?.[pose];
  if (!frame) return null;
  return (
    <img
      {...rest}
      className={css.root}
      src={frames[`../assets/figures/${id}/${pose}.png`]}
      width={frame.width * scale}
      height={frame.height * scale}
      alt=""
    />
  );
}
