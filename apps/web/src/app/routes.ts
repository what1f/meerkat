import { useSyncExternalStore, type ComponentType, type ReactNode } from 'react';
import type { TopBarProps } from '../shell';
import type { SheetClose } from '../scene';
import { TeamPage } from '../features/team';
import { SettingsPage } from '../features/settings';
import { OfficePage } from '../features/office';
import { MembersPage } from '../features/members';
import { OnboardingFlow } from '../features/onboarding';
export type Page =
  | { kind: 'members' }
  // A member opens over the page it was picked from.
  | { kind: 'member'; id: string; from?: 'office' }
  | { kind: 'team' }
  | { kind: 'office' }
  | { kind: 'settings' }
  | { kind: 'onboarding' }
  | { kind: 'design' }; // Only parsed and rendered in DEV; Vite erases its branch in production.
export type PageViewProps = {
  workspaceId: string;
  page: Page;
  navigate: (page: Page) => void;
  // Makes a team the current one without leaving the page.
  selectWorkspace: (id: string) => void;
  // Puts the page's top bar and body into the app shell.
  frame: (bar: FrameBar, body: ReactNode, board?: 'wall' | 'open') => ReactNode;
};
// A page over another one has no bar of its own; it says how it is put away.
export type FrameBar = TopBarProps & { close?: SheetClose };
export const pageViews: Partial<Record<Page['kind'], ComponentType<PageViewProps>>> = {
  members: MembersPage,
  member: MembersPage,
  team: TeamPage,
  settings: SettingsPage,
  office: OfficePage,
  onboarding: OnboardingFlow,
};
export function parseHash(hash: string): Page {
  const parts = hash.replace(/^#\/?/, '').split('/');
  const from = parts[0] === 'office' && parts[1] === 'member' ? 'office' : undefined;
  const [kind, id] = from ? parts.slice(1) : parts;
  if (kind === 'member' && id) {
    try {
      return from
        ? { kind, id: decodeURIComponent(id), from }
        : { kind, id: decodeURIComponent(id) };
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
  if (page.kind === 'member')
    return `#/${page.from ? `${page.from}/` : ''}member/${encodeURIComponent(page.id)}`;
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
