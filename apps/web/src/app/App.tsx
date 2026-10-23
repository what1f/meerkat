import { lazy, Suspense, useState, type ReactNode } from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { ToastProvider } from '../design';
import { I18nProvider, useT } from '../lib/i18n';
import { ThemeProvider } from '../lib/theme';
import { Sheet } from '../scene';
import { AppShell, Dock, TeamSwitcher, TopBar } from '../shell';
import { useWorkspaces } from '../lib/queries/workspaces';
import { useManagementEvents } from '../lib/events';
import { readStorage, writeStorage } from '../lib/storage';
import { navigate, pageViews, usePage, type FrameBar, type Page } from './routes';
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
  const hashPage = usePage();
  // A global subscription also includes workspaces created outside this page.
  useManagementEvents();
  const { data } = useWorkspaces({ request_id: 'list_workspaces' });
  // A first launch has no team: onboarding opens, and stays until it hands over.
  const [gate, setGate] = useState<'idle' | 'onboarding' | 'done'>('idle');
  if (gate === 'idle' && data?.workspaces.length === 0) setGate('onboarding');
  const page: Page = gate === 'onboarding' ? { kind: 'onboarding' } : hashPage;
  const go = (next: Page) => {
    if (gate === 'onboarding') setGate('done');
    navigate(next);
  };
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
  // A member is held up over the page it was opened from, which stays the active one.
  const under: Page = page.kind === 'member' ? { kind: page.from ?? 'members' } : page;
  const active = under.kind;
  const View = pageViews[under.kind];
  const selectWorkspace = (id: string) => {
    setSelected(id);
    writeStorage('workspace', id);
  };
  const title = pages.includes(active as (typeof pages)[number])
    ? t(`common.${active as (typeof pages)[number]}`)
    : undefined;
  // Putting a member away returns to where they were opened; reopening keeps that place.
  const sheetNavigate = (next: Page) =>
    go(
      next.kind === 'members'
        ? under
        : next.kind === 'member' && page.kind === 'member' && page.from
          ? { ...next, from: page.from }
          : next,
    );
  const Detail = page.kind === 'member' ? pageViews.member : undefined;
  const sheet = Detail && (
    <Detail
      workspaceId={current}
      page={page}
      navigate={sheetNavigate}
      selectWorkspace={selectWorkspace}
      frame={({ close }, body) => (
        <Sheet key={page.kind === 'member' ? page.id : ''} close={close}>
          {body}
        </Sheet>
      )}
    />
  );
  const frame = ({ close: _, ...bar }: FrameBar, body: ReactNode, board?: 'wall' | 'open') => (
    <AppShell
      board={board}
      dock={
        <Dock
          team={
            <TeamSwitcher
              data-testid="team-switcher"
              teams={teams}
              current={current}
              onSelect={(id) => {
                selectWorkspace(id);
                navigate({ kind: 'members' });
              }}
              onCreate={() => navigate({ kind: 'onboarding' })}
              createLabel={t('common.newTeam')}
              onSettings={() => navigate({ kind: 'settings' })}
              settingsLabel={t('common.settings')}
            />
          }
          items={(['members', 'team', 'office'] as const).map((kind) => ({
            id: kind,
            icon: kind === 'members' ? 'users' : kind === 'team' ? 'chat' : 'monitor',
            label: t(`common.${kind}`),
            active: active === kind,
            onClick: () => navigate({ kind }),
          }))}
        />
      }
      topBar={<TopBar {...bar} />}
      sheet={sheet}
    >
      {body}
    </AppShell>
  );
  return View ? (
    <View
      workspaceId={current}
      page={under}
      navigate={go}
      selectWorkspace={selectWorkspace}
      frame={frame}
    />
  ) : (
    frame({ title }, null)
  );
}
