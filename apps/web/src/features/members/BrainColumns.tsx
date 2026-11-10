import { useState } from 'react';
import {
  BrandMark,
  Icon,
  modelBrand,
  OptionColumn,
  SearchField,
  StatusDot,
  type Status,
} from '../../design';
import { Lever } from '../../scene';
import { useT } from '../../lib/i18n';
import { harnessName, levelLabel, named, sourceOf } from './labels';
import type { MemberDraftState } from './MemberConsole';

export type BrainDraft = Pick<
  MemberDraftState,
  'draft' | 'set' | 'machine' | 'machines' | 'harnesses' | 'models'
>;
import { isBrand } from './MemberCard';
import { defaultLevel, needsSignIn } from './harnessDraft';
import css from './MemberBench.module.css';

export const brainLevels = (m: BrainDraft) => {
  const { draft } = m;
  const harness = m.harnesses.find((h) => h.harness_id === draft.harness_id);
  const model = m.models.find((x) => x.model_id === draft.model);
  const levels = model?.supported_thinking_levels.length
    ? model.supported_thinking_levels
    : (harness?.supported_thinking_levels ?? []);
  return { harness, levels, level: Math.max(0, levels.indexOf(draft.thinking_level)) };
};

// Office, AI, model and thinking: each column narrows the next. Without
// `place` the office is already settled and only the AI is picked.
export function BrainColumns({
  m,
  place,
  onSignIn,
}: {
  m: BrainDraft;
  place?: Status;
  onSignIn: () => void;
}) {
  const t = useT();
  const [query, setQuery] = useState('');
  const { draft, machine } = m;
  // An offline Machine reports no AI; the workspace catalog stands in for it.
  const onMachine = machine?.harnesses.length ? machine.harnesses : m.harnesses;
  const { levels, level } = brainLevels(m);
  const signedOut = needsSignIn(machine, draft.harness_id);
  const thinks = !signedOut && levels.length > 0;
  const shown = m.models.filter((x) =>
    `${x.model_id} ${x.display_name}`.toLowerCase().includes(query.trim().toLowerCase()),
  );
  const online = (state: string) => state === 'running';
  // The selected place shows what the screen shows; the others show their own state.
  const dot = (x: { machine_id: string; observed_state: string }): Status =>
    x.machine_id === draft.machine_id && place
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
    <div className={css.brain} data-place={!!place || undefined}>
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
        {place && (
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
        )}
        <OptionColumn title={t('members.ai')}>
          {onMachine.map((x) => (
            <OptionColumn.Item
              key={x.harness_id}
              selected={x.harness_id === draft.harness_id}
              dimmed={x.availability !== 'available'}
              leading={isBrand(x.harness_id) ? <BrandMark brand={x.harness_id} size={16} /> : null}
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
            <OptionColumn.Item leading={<Icon name="plus" size={16} />} onSelect={onSignIn}>
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
  );
}
