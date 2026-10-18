import { isDesktopShell } from '../lib/desktopShell';
import type { ReactNode } from 'react';
import type { UIProps } from '../design/index';
import { Icon, LinkButton, Menu } from '../design';
import css from './TopBar.module.css';
export type TopBarProps = Omit<UIProps, 'className' | 'style'> & {
  title?: string;
  back?: { label: string; onClick: () => void } | { label: string; menu: ReactNode };
};
export function TopBar({ title, back, ...rest }: TopBarProps) {
  const drag = isDesktopShell() || undefined;
  return (
    <header {...rest} className={css.root} data-tauri-drag-region={drag}>
      {back &&
        ('menu' in back ? (
          <Menu trigger={<LinkButton disclosure>{back.label}</LinkButton>}>{back.menu}</Menu>
        ) : (
          <LinkButton onClick={back.onClick}>
            <Icon name="chevl" size={18} />
            {back.label}
          </LinkButton>
        ))}
      {title && <h1 data-tauri-drag-region={drag}>{title}</h1>}
    </header>
  );
}
