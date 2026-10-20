import { useState } from 'react';
import type * as DTO from '../../lib/api/generated';
import { useMachines } from '../../lib/queries/machines';
import { useHarnessTrials } from '../../lib/queries/onboarding';

export type HarnessTest = { state: 'run' | 'ok' | 'fail'; reply?: string };
export const firstModel = (harness: DTO.HarnessInfo | undefined) => {
  const models = harness?.model_catalog?.models ?? [];
  const model = models.find((m) => m.available) ?? models[0];
  return {
    model: model?.model_id ?? '',
    thinking_level: model?.supported_thinking_levels[0] ?? '',
  };
};

// The AI on this Mac: once the step opens, the latest pick, or else the first AI
// that is ready, is tested once so its row can show the reply.
export function useAiPick(request_id: string) {
  const [entered, setEntered] = useState(false);
  const [chosen, setChosen] = useState<string[]>([]);
  const catalog = useMachines({ request_id, workspace: null });
  const machine = catalog.data?.machines.find((m) => m.is_local);
  const harnesses = machine?.harnesses ?? [];
  const available = harnesses.filter((h) => h.availability === 'available');
  const pick = chosen.at(-1) ?? (entered ? available[0]?.harness_id : undefined);
  const tried = harnesses.filter((h) => h.harness_id === pick || chosen.includes(h.harness_id));
  const trials = useHarnessTrials(
    tried.map((harness) => ({
      machine_id: machine?.machine_id ?? '',
      harness_id: harness.harness_id,
      ...firstModel(harness),
      environment_variables: [],
      launch_arguments: [],
    })),
  );
  const tests: Record<string, HarnessTest> = Object.fromEntries(
    tried.map(({ harness_id }, index) => {
      const { data, isPending } = trials[index]!;
      const state = isPending ? 'run' : data?.state === 'passed' ? 'ok' : 'fail';
      return [harness_id, { state, reply: data?.reply }];
    }),
  );
  return {
    loading: catalog.isPending,
    loadFailed: catalog.isError && !catalog.data,
    reload: () => void catalog.refetch(),
    machine,
    available,
    pick,
    tests,
    choose: (harness_id: string) => {
      setEntered(true);
      setChosen((current) => [...current.filter((id) => id !== harness_id), harness_id]);
    },
    enter: () => setEntered(true),
    picked: harnesses.find((h) => h.harness_id === pick),
  };
}
