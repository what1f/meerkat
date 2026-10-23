import { isDesktopShell } from '../lib/desktopShell';
import type { UIProps } from '../design/index';
import css from './TopBar.module.css';
export type TopBarProps = Omit<UIProps, 'className' | 'style'> & {
  title?: string;
};
export function TopBar({ title, ...rest }: TopBarProps) {
  const drag = isDesktopShell() || undefined;
  return (
    <header {...rest} className={css.root} data-tauri-drag-region={drag}>
      {title && <h1 data-tauri-drag-region={drag}>{title}</h1>}
    </header>
  );
}
