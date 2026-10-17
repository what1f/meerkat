import type { UIProps } from './types';
import { icons, type IconName } from './icons';
import css from './Icon.module.css';
export type IconProps = Omit<UIProps<SVGSVGElement>, 'className' | 'style'> & {
  name: IconName;
  size?: 10 | 12 | 14 | 15 | 16 | 18 | 20 | 22;
};
export function Icon({ name, size = 16, ...rest }: IconProps) {
  const Glyph = icons[name];
  return <Glyph className={css.root} size={size} aria-hidden="true" {...rest} />;
}
