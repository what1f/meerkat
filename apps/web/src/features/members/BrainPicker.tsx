import { useState } from 'react';
import {
  BrandMark,
  OptionColumn,
  Popover,
  SearchField,
  StatusDot,
  type Status,
} from '../../design';
import { ConsoleScreen, Lever, ScreenReadout } from '../../scene';
import { t as translate, useT, type MessageKey } from '../../lib/i18n';
import type { MemberDraftState } from './MemberConsole';
import { isBrand, modelLabel } from './MemberCard';
import css from './MemberBench.module.css';

export const levelLabel = (level: string) => {
  const key = `members.levels.${level}` as MessageKey;
  try {
    return translate(key);
  } catch {
    return level;
  }
};

// Office, AI, model and thinking: each column narrows the next.
export function BrainPicker({
  m,
  status,
}: {
  m: MemberDraftState;
  workspace: string;
  status: Status | undefined;
}) {
  const t = useT();
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const { draft } = m;
  const machine = m.machine;
  const harnessOf = (id: string) => m.harnesses.find((h) => h.harness_id === id);
  // An offline Machine reports no AI; the workspace catalog stands in for it.
  const onMachine = machine?.harnesses.length ? machine.harnesses : m.harnesses;
  const harness = harnessOf(draft.harness_id);
  const model = m.models.find((x) => x.model_id === draft.model);
  const levels = model?.supported_thinking_levels.length
    ? model.supported_thinking_levels
    : (harness?.supported_thinking_levels ?? []);
  const level = Math.max(0, levels.indexOf(draft.thinking_level));
  const shown = m.models.filter((x) =>
    `${x.model_id} ${x.display_name}`.toLowerCase().includes(query.trim().toLowerCase()),
  );
  const online = (state: string) => state === 'running';
  const place: Status = status ?? (machine && online(machine.observed_state) ? 'ok' : 'idle');
  const pickModel = (id: string, supported: string[]) =>
    m.set({
      model: id,
      thinking_level: supported.includes(draft.thinking_level)
        ? draft.thinking_level
        : (supported[0] ?? ''),
    });

  return (
    <Popover
      open={open}
      onOpenChange={setOpen}
      trigger={
        <ConsoleScreen variant="readout" onClick={() => {}} data-testid="brain-picker">
          <ScreenReadout
            ai={{
              mark: isBrand(draft.harness_id) ? <BrandMark brand={draft.harness_id} /> : null,
              label: harness?.display_name ?? draft.harness_id,
            }}
            model={
              draft.model
                ? modelLabel(model?.display_name || draft.model)
                : t('members.defaultModel')
            }
            place={{ label: machine?.display_name ?? '', status: place }}
            level={{
              index: level,
              total: levels.length,
              label: levels.length ? levelLabel(levels[level]!) : '',
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
            const custom = v.trim();
            if (!custom) return;
            pickModel(custom, levels);
            setQuery('');
          }}
        />
        <div className={css.columns}>
          <OptionColumn title={t('members.place')}>
            {m.machines.map((x) => (
              <OptionColumn.Item
                key={x.machine_id}
                selected={x.machine_id === draft.machine_id}
                dimmed={!online(x.observed_state)}
                leading={<StatusDot status={online(x.observed_state) ? 'ok' : 'idle'} />}
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
                {x.display_name}
              </OptionColumn.Item>
            ))}
          </OptionColumn>
          <OptionColumn title={t('members.model')}>
            {shown.map((x) => (
              <OptionColumn.Item
                key={x.model_id}
                mono
                selected={x.model_id === draft.model}
                dimmed={!x.available}
                onSelect={() => pickModel(x.model_id, x.supported_thinking_levels)}
              >
                {x.display_name || x.model_id}
              </OptionColumn.Item>
            ))}
          </OptionColumn>
          <OptionColumn title={t('members.thinking')}>
            {levels.length > 0 && (
              <Lever
                levels={levels.map(levelLabel)}
                value={level}
                onChange={(i) => m.set({ thinking_level: levels[i] ?? '' })}
                aria-label={t('members.thinking')}
              />
            )}
          </OptionColumn>
        </div>
      </div>
    </Popover>
  );
}
