import { lazy, Suspense, useState } from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { ToastProvider } from '../design';
import { I18nProvider, useT } from '../lib/i18n';
import { ThemeProvider } from '../lib/theme';
import { AppShell, Sidebar, NavItem, TeamSwitcher, TopBar } from '../shell';
import { useWorkspaces } from '../lib/queries/workspaces';
import { useManagementEvents } from '../lib/events';
import { readStorage, writeStorage } from '../lib/storage';
import { navigate, usePage } from './routes';
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
  const current = teams.some((team) => team.id === selected) ? selected : (teams[0]?.id ?? '');
  const pages = ['members', 'team', 'office', 'settings'] as const;
  const active = page.kind === 'member' ? 'members' : page.kind;
  const title = pages.includes(active as (typeof pages)[number])
    ? t(`common.${active as (typeof pages)[number]}`)
    : undefined;
  return (
    <AppShell
      sidebar={
        <Sidebar
          header={
            <TeamSwitcher
              data-testid="team-switcher"
              teams={teams}
              current={current}
              onSelect={(id) => {
                setSelected(id);
                writeStorage('workspace', id);
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
      topBar={<TopBar title={title} />}
    >
      {null}
    </AppShell>
  );
}
