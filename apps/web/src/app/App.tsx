import { lazy, Suspense, useState, type ReactNode } from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { ToastProvider } from '../design';
import { I18nProvider, useT } from '../lib/i18n';
import { ThemeProvider } from '../lib/theme';
import { AppShell, Sidebar, NavItem, TeamSwitcher, TopBar, type TopBarProps } from '../shell';
import { useWorkspaces } from '../lib/queries/workspaces';
import { useManagementEvents } from '../lib/events';
import { readStorage, writeStorage } from '../lib/storage';
import { navigate, pageViews, usePage } from './routes';
const queryClient = new QueryClient();
const DesignPreview = import.meta.env.DEV ? lazy(() => import('../dev/DesignPreview')) : null;
export function App() {
  const page = usePage();
  return (
    <QueryClientProvider client={queryClient}>
      <I18nProvider>
        <ThemeProvider>
          <ToastProvider>
            {import.meta.env.DEV && page.kind === 'design' && DesignPreview ? (
              <Suspense>
                <DesignPreview />
              </Suspense>
            ) : (
              <WorkspaceShell />
            )}
          </ToastProvider>
        </ThemeProvider>
      </I18nProvider>
    </QueryClientProvider>
  );
}

function WorkspaceShell() {
  const t = useT();
  const page = usePage();
  // A global subscription also includes workspaces created outside this page.
  useManagementEvents();
  const { data } = useWorkspaces({ request_id: 'list_workspaces' });
  const [selected, setSelected] = useState(() =>
    readStorage('workspace', '', (v): v is string => typeof v === 'string'),
  );
  const teams = (data?.workspaces ?? []).flatMap((workspace) =>
    workspace.workspace ? [{ id: workspace.workspace.workspace_id, name: workspace.name }] : [],
  );
  // Without a valid selection the first team is pinned, so a team that appears
  // later cannot take the current place. Deleting the selected one pins again.
  const valid = teams.some((team) => team.id === selected);
  const first = teams[0]?.id;
  if (!valid && first) setSelected(first);
  const current = valid ? selected : (first ?? '');
  const pages = ['members', 'team', 'office', 'settings'] as const;
  const active = page.kind === 'member' ? 'members' : page.kind;
  const View = pageViews[page.kind];
  const selectWorkspace = (id: string) => {
    setSelected(id);
    writeStorage('workspace', id);
  };
  const title = pages.includes(active as (typeof pages)[number])
    ? t(`common.${active as (typeof pages)[number]}`)
    : undefined;
  const frame = (bar: TopBarProps, body: ReactNode) => (
    <AppShell
      sidebar={
        <Sidebar
          header={
            <TeamSwitcher
              data-testid="team-switcher"
              teams={teams}
              current={current}
              onSelect={(id) => {
                selectWorkspace(id);
                navigate({ kind: 'members' });
              }}
              onCreate={() => navigate({ kind: 'onboarding' })}
              createLabel={t('common.create')}
            />
          }
          nav={pages
            .filter((kind) => kind !== 'settings')
            .map((kind) => (
              <NavItem
                key={kind}
                icon={kind === 'members' ? 'users' : kind === 'team' ? 'chat' : 'monitor'}
                label={t(`common.${kind}`)}
                active={active === kind}
                onClick={() => navigate({ kind })}
                testId={`nav-${kind}`}
              />
            ))}
          footer={
            <NavItem
              icon="settings"
              label={t('common.settings')}
              active={active === 'settings'}
              onClick={() => navigate({ kind: 'settings' })}
              testId="nav-settings"
            />
          }
        />
      }
      topBar={<TopBar {...bar} />}
    >
      {body}
    </AppShell>
  );
  return View ? (
    <View
      workspaceId={current}
      page={page}
      navigate={navigate}
      selectWorkspace={selectWorkspace}
      frame={frame}
    />
  ) : (
    frame({ title }, null)
  );
}
