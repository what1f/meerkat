import { PullCord } from '../scene';
import { isDesktopShell } from '../lib/desktopShell';
import { setTheme } from '../lib/theme';
import { IconButton } from '../design';
import { useT } from '../lib/i18n';
import { useSidebarLayout } from './useSidebarLayout';
import type { ReactNode } from 'react';
import type { UIProps } from '../design/index';

import css from './AppShell.module.css';
export type AppShellProps = Omit<UIProps, 'className' | 'style'> & {
  sidebar: ReactNode;
  topBar: ReactNode;
  children: ReactNode;
  // 'wall': the pegboard runs under the transparent top bar; 'open': content starts at the top.
  board?: 'wall' | 'open';
};
export function AppShell({ sidebar, topBar, children, board, ...rest }: AppShellProps) {
  const t = useT();
  const { hidden, rail, settling, resizing, setHidden } = useSidebarLayout();
  const desktop = isDesktopShell();
  return (
    <div
      {...rest}
      className={css.root}
      data-board={board}
      data-desktop={desktop}
      data-settle={settling || undefined}
      data-resizing={resizing || undefined}
    >
      <div data-rail={rail}>
        {desktop && <div aria-hidden="true" data-desktop-safe-area data-tauri-drag-region />}
        <IconButton
          icon="sidebar"
          label={t('common.toggleSidebar')}
          onClick={() => setHidden(!hidden)}
        />
        {sidebar}
      </div>
      <div>
        {topBar}
        <PullCord
          label={t('common.theme')}
          onToggle={() =>
            setTheme(document.documentElement.dataset.theme === 'dark' ? 'light' : 'dark')
          }
        />
        <main>{children}</main>
      </div>
    </div>
  );
}
