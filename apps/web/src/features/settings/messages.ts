import { registerMessages } from '../../lib/i18n';
const en = {
  language: 'Language',
  system: 'System',
  light: 'Light',
  dark: 'Dark',
  // Each language is named in itself, so both catalogs read the same.
  zh: '简体中文',
  en: 'English',
};
declare module '../../lib/i18n' {
  interface MessageCatalog {
    settings: typeof en;
  }
}
registerMessages('settings', {
  en,
  zh: {
    language: '语言',
    system: '跟随系统',
    light: '浅色',
    dark: '深色',
    zh: '简体中文',
    en: 'English',
  },
});
