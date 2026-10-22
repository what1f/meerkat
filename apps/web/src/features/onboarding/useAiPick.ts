import { useState } from 'react';
import type * as DTO from '../../lib/api/generated';
import { useMachines } from '../../lib/queries/machines';

export const firstModel = (harness: DTO.HarnessInfo | undefined) => {
  const models = harness?.model_catalog?.models ?? [];
  const model = models.find((m) => m.available) ?? models[0];
  return {
    model: model?.model_id ?? '',
    thinking_level: model?.supported_thinking_levels[0] ?? '',
  };
};

export function useAiPick(request_id: string) {
  const [entered, setEntered] = useState(false);
  const [chosen, setChosen] = useState<string>();
  const catalog = useMachines({ request_id, workspace: null });
  const machine = catalog.data?.machines.find((m) => m.is_local);
  const harnesses = machine?.harnesses ?? [];
  const available = harnesses.filter((h) => h.availability === 'available');
  const pick = chosen ?? (entered ? available[0]?.harness_id : undefined);
  return {
    loading: catalog.isPending,
    loadFailed: catalog.isError && !catalog.data,
    reload: () => void catalog.refetch(),
    machine,
    available,
    pick,
    choose: (harness_id: string) => {
      setEntered(true);
      setChosen(harness_id);
    },
    enter: () => setEntered(true),
    picked: harnesses.find((h) => h.harness_id === pick),
  };
}
