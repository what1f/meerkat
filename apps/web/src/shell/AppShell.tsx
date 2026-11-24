import { PullCord } from '../scene';
import { isMacDesktopShell } from '../lib/desktopShell';
import { setTheme } from '../lib/theme';
import { useT } from '../lib/i18n';
import type { ReactNode } from 'react';
import type { UIProps } from '../design/index';
import css from './AppShell.module.css';

export type AppShellProps = Omit<UIProps, 'className' | 'style'> & {
  dock: ReactNode;
  topBar: ReactNode;
  children: ReactNode;
  board?: 'wall' | 'open';
  // Held up over the page, which stays in view but out of reach.
  sheet?: ReactNode;
};
export function AppShell({ dock, topBar, children, board, sheet, ...rest }: AppShellProps) {
  const t = useT();
  return (
    <div
      {...rest}
      className={css.root}
      data-board={board}
      data-desktop={isMacDesktopShell()}
      data-sheet={sheet ? '' : undefined}
    >
      {topBar}
      <PullCord
        label={t('common.theme')}
        onToggle={() =>
          setTheme(document.documentElement.dataset.theme === 'dark' ? 'light' : 'dark')
        }
      />
      <main inert={!!sheet}>{children}</main>
      {sheet}
      {dock}
    </div>
  );
}
