import type { UIProps } from '../design/index';
import peek from '../assets/mascot/meerkat-peek.png';
import css from './Mascot.module.css';
export type MascotProps = Omit<UIProps, 'className' | 'style'> & { pose: 'peek' };
export function Mascot({ pose, ...rest }: MascotProps) {
  return <img {...rest} className={css.root} data-pose={pose} src={peek} alt="" />;
}
