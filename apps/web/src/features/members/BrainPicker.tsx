import { useState } from 'react';
import {
  BrandMark,
  Icon,
  modelBrand,
  OptionColumn,
  Popover,
  SearchField,
  StatusDot,
  type Status,
} from '../../design';
import { ConsoleScreen, Lever, ScreenReadout } from '../../scene';
import { useT } from '../../lib/i18n';
import { harnessName, levelLabel, named, sourceOf } from './labels';
import type { MemberDraftState } from './MemberConsole';
import { isBrand, useModelFace } from './MemberCard';
import { BuiltinSignInDialog } from '../builtin-agent';
import { defaultLevel, needsSignIn } from './harnessDraft';
import css from './MemberBench.module.css';

// Office, AI, model and thinking: each column narrows the next.
export function BrainPicker({
  m,
  workspace,
  status,
}: {
  m: MemberDraftState;
  workspace: string;
  status: Status | undefined;
}) {
  const t = useT();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [signing, setSigning] = useState(false);
  const { draft } = m;
  const machine = m.machine;
  const harnessOf = (id: string) => m.harnesses.find((h) => h.harness_id === id);
  // An offline Machine reports no AI; the workspace catalog stands in for it.
  const onMachine = machine?.harnesses.length ? machine.harnesses : m.harnesses;
  const harness = harnessOf(draft.harness_id);
  const model = m.models.find((x) => x.model_id === draft.model);
  const face = useModelFace(workspace, draft.machine_id, draft.harness_id, draft.model);
  const levels = model?.supported_thinking_levels.length
    ? model.supported_thinking_levels
    : (harness?.supported_thinking_levels ?? []);
  const level = Math.max(0, levels.indexOf(draft.thinking_level));
  const signedOut = needsSignIn(machine, draft.harness_id);
  const thinks = !signedOut && levels.length > 0;
  const shown = m.models.filter((x) =>
    `${x.model_id} ${x.display_name}`.toLowerCase().includes(query.trim().toLowerCase()),
  );
  const online = (state: string) => state === 'running';
  const place: Status = status ?? (machine && online(machine.observed_state) ? 'ok' : 'idle');
  // The selected place shows what the screen shows; the others show their own state.
  const dot = (x: { machine_id: string; observed_state: string }): Status =>
    x.machine_id === draft.machine_id
      ? place
      : x.observed_state === 'running'
        ? 'ok'
        : x.observed_state === 'error'
          ? 'err'
          : x.observed_state === 'starting'
            ? 'busy'
            : 'idle';
  const twins = (x: { model_id: string; display_name: string }) =>
    m.models.filter((y) => named(y) === named(x)).length > 1;
  const pickModel = (id: string, supported: string[]) =>
    m.set({
      model: id,
      thinking_level: supported.includes(draft.thinking_level)
        ? draft.thinking_level
        : defaultLevel(supported),
    });

  return (
    <>
      <Popover
        open={open}
        onOpenChange={setOpen}
        trigger={
          <ConsoleScreen variant="readout" onClick={() => {}} data-testid="brain-picker">
            <ScreenReadout
              ai={{
                mark: isBrand(draft.harness_id) ? <BrandMark brand={draft.harness_id} /> : null,
                label: harnessName(harness, draft.harness_id),
              }}
              model={signedOut ? { label: t('builtinAgent.connectModel'), mark: null } : face}
              place={{ label: machine?.display_name ?? '', status: place }}
              level={{
                index: level,
                total: signedOut ? 0 : levels.length,
                label: !signedOut && levels.length ? levelLabel(levels[level]!) : '',
              }}
            />
          </ConsoleScreen>
        }
      >
        <div className={css.brain}>
          <SearchField
            value={query}
            onChange={setQuery}
            placeholder={t('members.searchModel')}
            autoFocus
            onEnter={(v) => {
              if (signedOut) return;
              const custom = v.trim();
              if (!custom) return;
              pickModel(custom, levels);
              setQuery('');
            }}
          />
          <div className={css.columns} data-thinks={thinks || undefined}>
            <OptionColumn title={t('members.place')}>
              {m.machines.map((x) => (
                <OptionColumn.Item
                  key={x.machine_id}
                  selected={x.machine_id === draft.machine_id}
                  dimmed={!online(x.observed_state)}
                  leading={<StatusDot status={dot(x)} />}
                  onSelect={() => m.set({ machine_id: x.machine_id })}
                >
                  {x.display_name}
                </OptionColumn.Item>
              ))}
            </OptionColumn>
            <OptionColumn title={t('members.ai')}>
              {onMachine.map((x) => (
                <OptionColumn.Item
                  key={x.harness_id}
                  selected={x.harness_id === draft.harness_id}
                  dimmed={x.availability !== 'available'}
                  leading={
                    isBrand(x.harness_id) ? <BrandMark brand={x.harness_id} size={16} /> : null
                  }
                  onSelect={() =>
                    x.harness_id !== draft.harness_id &&
                    m.set({ harness_id: x.harness_id, model: '', thinking_level: '' })
                  }
                >
                  {harnessName(x, x.harness_id)}
                </OptionColumn.Item>
              ))}
            </OptionColumn>
            <OptionColumn title={t('members.model')}>
              {signedOut ? (
                <OptionColumn.Item
                  leading={<Icon name="plus" size={16} />}
                  onSelect={() => {
                    setOpen(false);
                    setSigning(true);
                  }}
                >
                  {t('builtinAgent.connectModel')}
                </OptionColumn.Item>
              ) : (
                shown.map((x) => {
                  const brand = modelBrand(x.model_id, x.display_name);
                  return (
                    <OptionColumn.Item
                      key={x.model_id}
                      mono={!x.display_name}
                      leading={brand ? <BrandMark brand={brand} size={16} /> : null}
                      selected={x.model_id === draft.model}
                      dimmed={!x.available}
                      hint={twins(x) ? sourceOf(x.model_id) : undefined}
                      onSelect={() => pickModel(x.model_id, x.supported_thinking_levels)}
                    >
                      {named(x)}
                    </OptionColumn.Item>
                  );
                })
              )}
            </OptionColumn>
            {thinks && (
              <OptionColumn
                title={t('members.thinking')}
                control={
                  <Lever
                    levels={levels.map(levelLabel)}
                    value={level}
                    onChange={(i) => m.set({ thinking_level: levels[i] ?? '' })}
                    aria-label={t('members.thinking')}
                  />
                }
              />
            )}
          </div>
        </div>
      </Popover>
      {machine && signing && (
        <BuiltinSignInDialog
          machine={machine}
          open={signing}
          onClose={() => setSigning(false)}
          onSignedIn={() => m.set({ model: '', thinking_level: '' })}
        />
      )}
    </>
  );
}
