import { TrayField, TextInput } from '../../design';
import { useT } from '../../lib/i18n';
import type { MemberDraftState } from './MemberConsole';
import css from './MemberBench.module.css';

// Environment variables, launch arguments and the command path.
export function AdvancedDrawer({ m }: { m: MemberDraftState }) {
  const t = useT();
  const { draft } = m;
  const rows = [...draft.environment_variables, { name: '', value: '' }];
  const setRow = (index: number, patch: Partial<{ name: string; value: string }>) =>
    m.set({
      environment_variables: rows
        .map((row, i) => (i === index ? { ...row, ...patch } : row))
        .filter((row) => row.name || row.value),
    });
  return (
    <div className={css.advanced}>
      <TrayField label={t('members.env')}>
        {rows.map((row, i) => (
          <div key={i} className={css.pair}>
            <TextInput
              size="sm"
              mono
              value={row.name}
              onChange={(name) => setRow(i, { name })}
              placeholder={t('members.envName')}
              aria-label={t('members.envName')}
            />
            <TextInput
              size="sm"
              mono
              value={row.value}
              onChange={(value) => setRow(i, { value })}
              placeholder={t('members.envValue')}
              aria-label={t('members.envValue')}
            />
          </div>
        ))}
      </TrayField>
      <TrayField label={t('members.args')}>
        <TextInput
          size="sm"
          mono
          value={m.argsText}
          onChange={m.setArgs}
          placeholder="--verbose"
          aria-label={t('members.args')}
        />
      </TrayField>
      <TrayField label={t('members.cliPath')}>
        <TextInput
          size="sm"
          mono
          value={draft.cli_path}
          onChange={(cli_path) => m.set({ cli_path })}
          placeholder={t('members.auto')}
          aria-label={t('members.cliPath')}
        />
      </TrayField>
    </div>
  );
}
