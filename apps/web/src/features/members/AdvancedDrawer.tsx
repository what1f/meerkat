import { Button, Group, StatusDot, TextInput } from '../../design';
import { useT } from '../../lib/i18n';
import { useTestHarness } from '../../lib/queries/catalogs';
import type { MemberDraftState } from './MemberConsole';
import css from './MemberBench.module.css';

// Environment variables, launch arguments and the command path, tried out before saving.
export function AdvancedDrawer({ m }: { m: MemberDraftState }) {
  const t = useT();
  const test = useTestHarness();
  const { draft } = m;
  const rows = [...draft.environment_variables, { name: '', value: '' }];
  const setRow = (index: number, patch: Partial<{ name: string; value: string }>) =>
    m.set({
      environment_variables: rows
        .map((row, i) => (i === index ? { ...row, ...patch } : row))
        .filter((row) => row.name || row.value),
    });
  const result = test.data;
  const passed = result?.state === 'passed';
  return (
    <div className={css.advanced}>
      <Group>
        <Group.Row label={t('members.env')} align="top">
          <div className={css.env}>
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
          </div>
        </Group.Row>
        <Group.Row label={t('members.args')}>
          <div className={css.field}>
            <TextInput
              size="sm"
              mono
              value={m.argsText}
              onChange={m.setArgs}
              placeholder="--verbose"
              aria-label={t('members.args')}
            />
          </div>
        </Group.Row>
        <Group.Row label={t('members.cliPath')}>
          <div className={css.field}>
            <TextInput
              size="sm"
              mono
              value={draft.cli_path}
              onChange={(cli_path) => m.set({ cli_path })}
              placeholder={t('members.auto')}
              aria-label={t('members.cliPath')}
            />
          </div>
        </Group.Row>
      </Group>
      <div className={css.trial}>
        <Button
          size="sm"
          loading={test.isPending}
          disabled={!draft.machine_id || !draft.harness_id}
          leading={
            result || test.isError ? (
              <StatusDot
                status={passed ? 'ok' : 'err'}
                tip={result?.reply || (passed ? t('members.testPassed') : t('members.testFailed'))}
              />
            ) : undefined
          }
          onClick={() =>
            test.mutate({
              machine_id: draft.machine_id,
              harness_id: draft.harness_id,
              model: draft.model,
              thinking_level: draft.thinking_level,
              cli_path: draft.cli_path || undefined,
              environment_variables: draft.environment_variables,
              launch_arguments: draft.launch_arguments,
            })
          }
          data-testid="member-test"
        >
          {t('members.test')}
        </Button>
      </div>
    </div>
  );
}
