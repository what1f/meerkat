import { createElement, Fragment, useEffect, useSyncExternalStore, type ReactNode } from 'react';
import { readStorage, writeStorage } from './storage';
export type Theme = 'system' | 'light' | 'dark';
const valid = (v: unknown): v is Theme => v === 'system' || v === 'light' || v === 'dark';
let theme = readStorage<Theme>('theme', 'system', valid);
const listeners = new Set<() => void>();
const media = matchMedia('(prefers-color-scheme: dark)');
function apply() {
  document.documentElement.dataset.theme =
    theme === 'system' ? (media.matches ? 'dark' : 'light') : theme;
}
export function setTheme(next: Theme) {
  theme = next;
  writeStorage('theme', next);
  apply();
  listeners.forEach((fn) => fn());
}
function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
export function useTheme() {
  return useSyncExternalStore(subscribe, () => theme);
}
export function ThemeProvider({ children }: { children: ReactNode }) {
  useTheme();
  useEffect(() => {
    apply();
    media.addEventListener('change', apply);
    return () => media.removeEventListener('change', apply);
  }, []);
  return createElement(Fragment, null, children);
}
