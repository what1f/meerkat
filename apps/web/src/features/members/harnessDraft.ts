import type * as DTO from '../../lib/api/generated';
import { parseArgs } from './args';
import type { Draft } from './draft';

export function harnessDraft(
  base: Draft,
  edits: Partial<Draft>,
  argsText: string | null,
  id: string | null,
  machines: DTO.MachineCatalogItem[],
  harnesses: DTO.HarnessCatalogItem[],
) {
  const merged = { ...base, ...edits };
  if (argsText !== null) merged.launch_arguments = parseArgs(argsText);
  const list = machines.filter((m) => m.revoked_at === null);
  const machine =
    list.find((m) => m.machine_id === merged.machine_id) ??
    (id ? undefined : (list.find((m) => m.is_local) ?? list[0]));
  const harness =
    merged.harness_id ||
    (harnesses.find((h) => h.availability === 'available') ?? harnesses[0])?.harness_id ||
    '';
  const draft: Draft = {
    ...merged,
    machine_id: machine?.machine_id ?? merged.machine_id,
    harness_id: harness,
  };
  const defaults =
    machine?.harnesses.find((h) => h.harness_id === harness) ??
    harnesses.find((h) => h.harness_id === harness);
  if (!id) {
    if (edits.environment_variables === undefined)
      draft.environment_variables = defaults?.default_environment_variables ?? [];
    if (edits.launch_arguments === undefined && argsText === null)
      draft.launch_arguments = defaults?.default_launch_arguments ?? [];
  }
  return { draft, list, machine, harness };
}

export function harnessDefaults(
  id: string,
  machine: DTO.MachineCatalogItem | undefined,
  catalog: DTO.HarnessCatalogItem[] = [],
) {
  const selected =
    machine?.harnesses.find((h) => h.harness_id === id) ?? catalog.find((h) => h.harness_id === id);
  return {
    environment_variables: selected?.default_environment_variables ?? [],
    launch_arguments: selected?.default_launch_arguments ?? [],
  };
}
