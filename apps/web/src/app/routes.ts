import { useSyncExternalStore, type ComponentType, type ReactNode } from 'react';
import type { TopBarProps } from '../shell';
import { TeamPage } from '../features/team';
import { SettingsPage } from '../features/settings';
export type Page =
  | { kind: 'members' }
  | { kind: 'member'; id: string }
  | { kind: 'team' }
  | { kind: 'office' }
  | { kind: 'settings' }
  | { kind: 'onboarding' }
  | { kind: 'design' }; // Only parsed and rendered in DEV; Vite erases its branch in production.
export type PageViewProps = {
  workspaceId: string;
  page: Page;
  navigate: (page: Page) => void;
  // Puts the page's top bar and body into the app shell.
  frame: (bar: TopBarProps, body: ReactNode) => ReactNode;
};
export const pageViews: Partial<Record<Page['kind'], ComponentType<PageViewProps>>> = {
  team: TeamPage,
  settings: SettingsPage,
};
export function parseHash(hash: string): Page {
  const [kind, id] = hash.replace(/^#\/?/, '').split('/');
  if (kind === 'member' && id) {
    try {
      return { kind, id: decodeURIComponent(id) };
    } catch {
      return { kind: 'members' };
    }
  }
  if (kind === 'team' || kind === 'office' || kind === 'settings' || kind === 'onboarding')
    return { kind };
  if (import.meta.env.DEV && kind === 'design') return { kind };
  return { kind: 'members' };
}
export function pageHash(page: Page): string {
  if (page.kind === 'member') return `#/member/${encodeURIComponent(page.id)}`;
  if (page.kind === 'design') return import.meta.env.DEV ? '#/design' : '#/members';
  return `#/${page.kind}`;
}
export function navigate(page: Page) {
  location.hash = pageHash(page);
}
function subscribe(listener: () => void) {
  addEventListener('hashchange', listener);
  return () => removeEventListener('hashchange', listener);
}
export function usePage(): Page {
  return parseHash(useSyncExternalStore(subscribe, () => location.hash));
}
