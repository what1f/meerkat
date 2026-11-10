import { useState } from 'react';
import type * as DTO from '../../lib/api/generated';
import { useHarnessCatalog, useModelCatalog } from '../../lib/queries/catalogs';
import { useMachines } from '../../lib/queries/machines';
import { defaultLevel, harnessDraft, needsSignIn } from './harnessDraft';
import { blank, pick, type Draft } from './draft';

/** Just the AI a member runs, picked apart from the rest of their profile. */
export function useBrainDraft(agent: DTO.Agent, seed: Partial<Draft>) {
  const ref = { workspace_id: agent.agent?.workspace_id ?? '' };
  const machines = useMachines({ request_id: 'brain', workspace: ref });
  const harnesses = useHarnessCatalog({ request_id: 'brain', workspace: ref });
  const [edits, setEdits] = useState(seed);
  const { draft, list, machine, harness } = harnessDraft(
    { ...blank, ...pick(agent) },
    edits,
    null,
    agent.agent?.agent_id ?? null,
    machines.data?.machines ?? [],
    harnesses.data?.harnesses ?? [],
  );
  const models = useModelCatalog(
    { request_id: 'brain', workspace: ref, machine_id: draft.machine_id, harness_id: harness },
    !!draft.machine_id && !!harness && !needsSignIn(machine, harness),
    machine?.harnesses.find((h) => h.harness_id === harness)?.model_catalog,
  );
  if (!draft.model) {
    const first = models.data?.models.find((m) => m.available) ?? models.data?.models[0];
    draft.model = first?.model_id ?? '';
    draft.thinking_level ||= defaultLevel(first?.supported_thinking_levels ?? []);
  }
  return {
    draft,
    set: (patch: Partial<Draft>) => setEdits((current) => ({ ...current, ...patch })),
    machine,
    machines: list,
    harnesses: harnesses.data?.harnesses ?? [],
    models: models.data?.models ?? [],
  };
}
