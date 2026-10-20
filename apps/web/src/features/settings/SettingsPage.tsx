import type { ReactNode } from 'react';
import { Group, Select, ThemeThumb } from '../../design';
import { setLanguage, useLanguage, useT, type Language } from '../../lib/i18n';
import { setTheme, useTheme, type Theme } from '../../lib/theme';
import css from './SettingsPage.module.css';
import './messages';

const languages = ['zh', 'en'] as const satisfies readonly Language[];
const themes = ['system', 'light', 'dark'] as const satisfies readonly Theme[];

export type SettingsPageProps = {
  frame: (bar: { title: string }, body: ReactNode) => ReactNode;
};

export function SettingsPage({ frame }: SettingsPageProps) {
  const t = useT();
  const language = useLanguage();
  const theme = useTheme();
  return frame(
    { title: t('common.settings') },
    <div className={css.page}>
      <Group>
        <Group.Row label={t('settings.language')}>
          <Select
            data-testid="settings-language"
            aria-label={t('settings.language')}
            options={languages.map((value) => ({ value, label: t(`settings.${value}`) }))}
            value={language}
            onChange={setLanguage}
          />
        </Group.Row>
        <Group.Row label={t('common.theme')} align="top">
          <div className={css.thumbs}>
            {themes.map((mode) => (
              <ThemeThumb
                key={mode}
                data-testid={`settings-theme-${mode}`}
                mode={mode}
                selected={theme === mode}
                label={t(`settings.${mode}`)}
                onSelect={() => setTheme(mode)}
              />
            ))}
          </div>
        </Group.Row>
      </Group>
    </div>,
  );
}
