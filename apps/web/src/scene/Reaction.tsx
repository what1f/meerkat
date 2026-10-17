import type { UIProps } from '../design/index';
const images = import.meta.glob<string>('../assets/reactions/*.png', {
  eager: true,
  query: '?url',
  import: 'default',
});
import css from './Reaction.module.css';
export type ReactionProps = Omit<UIProps, 'className' | 'style'> & { id: string; size?: number };
export function Reaction({ id, size, ...rest }: ReactionProps) {
  const src = images[`../assets/reactions/${id}.png`];
  return src ? (
    <img {...rest} className={css.root} src={src} width={size} height={size} alt="" />
  ) : null;
}
