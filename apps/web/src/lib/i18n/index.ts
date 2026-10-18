import { createElement, Fragment, useEffect, useSyncExternalStore, type ReactNode } from 'react';
import { readStorage, writeStorage } from '../storage';
import { common } from './common';
export type Language = 'zh' | 'en';
// Features augment this interface with their own message shapes, then register messages.
export interface MessageCatalog {
  common: typeof common.en;
}
type Paths<T> = {
  [K in keyof T & string]: T[K] extends string ? K : `${K}.${Paths<T[K]>}`;
}[keyof T & string];
export type MessageKey = Paths<MessageCatalog>;
type Translated<T> = { [K in keyof T]: T[K] extends string ? string : Translated<T[K]> };
const dictionaries: Record<string, { en: unknown; zh: unknown }> = { common };
const listeners = new Set<() => void>();
const isLanguage = (v: unknown): v is Language => v === 'zh' || v === 'en';
let language = readStorage<Language>(
  'language',
  navigator.language.startsWith('zh') ? 'zh' : 'en',
  isLanguage,
);
export function registerMessages<K extends keyof MessageCatalog>(
  namespace: K,
  messages: { en: Translated<MessageCatalog[K]>; zh: Translated<MessageCatalog[K]> },
) {
  dictionaries[namespace] = messages;
}
export function setLanguage(next: Language) {
  language = next;
  document.documentElement.lang = next;
  writeStorage('language', next);
  listeners.forEach((fn) => fn());
}
function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
export function useLanguage() {
  return useSyncExternalStore(subscribe, () => language);
}
// One function per language, so memoized components re-render their text when it changes.
const translators: Record<Language, typeof t> = { zh: (key) => t(key), en: (key) => t(key) };
export function useT() {
  return translators[useLanguage()];
}
export function t(key: MessageKey): string {
  const [namespace, ...path] = key.split('.');
  let value: unknown = dictionaries[namespace ?? '']?.[language];
  for (const part of path)
    value =
      value && typeof value === 'object' ? (value as Record<string, unknown>)[part] : undefined;
  if (typeof value !== 'string') throw new Error(`Missing message: ${key}`);
  return value;
}
export function I18nProvider({ children }: { children: ReactNode }) {
  const active = useLanguage();
  useEffect(() => {
    document.documentElement.lang = active;
  }, [active]);
  return createElement(Fragment, null, children);
}
