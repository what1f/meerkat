import type { ReactNode } from 'react';
import type { UIProps } from '../design/index';
import { useSidebarLayout } from './useSidebarLayout';
import { SidebarGrip } from './SidebarGrip';
import css from './Sidebar.module.css';
export type SidebarProps = Omit<UIProps, 'className' | 'style'> & {
  header: ReactNode;
  nav: ReactNode;
  footer: ReactNode;
};
export function Sidebar({ header, nav, footer, ...rest }: SidebarProps) {
  const { width, rail, hidden } = useSidebarLayout();
  return (
    <aside
      {...rest}
      className={css.root}
      hidden={hidden}
      data-rail={rail}
      style={{ width: rail ? 60 : width }}
    >
      <header>{header}</header>
      <nav>{nav}</nav>
      <footer>{footer}</footer>
      <SidebarGrip />
    </aside>
  );
}
