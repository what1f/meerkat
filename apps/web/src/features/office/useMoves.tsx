import { useState, type ComponentType } from 'react';
import type * as DTO from '../../lib/api/generated';
import { useToast } from '../../design';
import { useCarry } from '../../scene';
import { useUpdateAgent } from '../../lib/queries/agents';
import { useT } from '../../lib/i18n';
import { useMoveFailures } from './useMoveFailures';
import { move, retryArrival, type Brain } from './move';
import './messages';

// Picks the AI a member runs on the computer they were dropped on; the app supplies it.
export type MoveDialog = ComponentType<{
  agent: DTO.Agent;
  target: DTO.MachineCatalogItem;
  open: boolean;
  onClose: () => void;
  onConfirm: (brain: Brain) => void;
}>;

/** Carrying members between computers: where each one sits, and what their moving box offers. */
export function useMoves(
  agents: DTO.Agent[] | undefined,
  machines: DTO.MachineCatalogItem[],
  MoveMemberDialog: MoveDialog,
) {
  const t = useT();
  const toast = useToast();
  const update = useUpdateAgent();
  const [arrived, setArrived] = useState<string | null>(null);
  // The drop stays named while its dialog animates out; `n` starts each drop afresh.
  const [drop, setDrop] = useState<{ agent?: DTO.Agent; target?: DTO.MachineCatalogItem }>({});
  const [open, setOpen] = useState(false);
  const [n, setN] = useState(0);
  const send = (request: DTO.UpdateAgentRequest) =>
    update.mutate(request, { onError: () => toast({ message: t('office.moveFailed') }) });
  const carry = useCarry((id, machine) => {
    const agent = agents?.find((a) => a.agent?.agent_id === id);
    const target = machines.find((m) => m.machine_id === machine);
    if (!agent?.agent || !target) return;
    setDrop({ agent, target });
    setN(n + 1);
    setOpen(true);
  });
  useMoveFailures(agents, (agent, ref, state) => {
    if (state === 'stalled') return toast({ message: t('office.arrivalFailed') });
    toast({ message: t('office.moveFailed') });
    update.mutate(move(ref, agent.machine_id));
  });
  // Until the catalog refetches, a member who was just carried stays where they were dropped.
  const pending = update.isPending ? update.variables : undefined;
  // Where the member's files are until they have started on the new computer.
  const homeOf = (agent: DTO.Agent) =>
    agent.relocation && agent.relocation.state !== 'failed'
      ? agent.relocation.source_machine_id
      : agent.machine_id;
  // A member on the move already sits at the new computer, boxed up until they start there.
  const placeOf = (agent: DTO.Agent) =>
    (pending?.machine_id && pending.agent?.agent_id === agent.agent?.agent_id
      ? pending.machine_id
      : undefined) ??
    (agent.relocation && agent.relocation.state !== 'failed'
      ? agent.relocation.machine_id
      : agent.machine_id);
  const online = (id: string) =>
    machines.some((m) => m.machine_id === id && m.observed_state === 'running');
  const moveOf = (agent: DTO.Agent) => {
    const ref = agent.agent;
    const [home, place] = [homeOf(agent), placeOf(agent)];
    if (!ref || home === place) return undefined;
    const back = () => send(move(ref, home));
    switch (agent.relocation?.state) {
      case 'stalled':
        return { stuck: true, onRetry: () => send(retryArrival(ref)), onReturn: back };
      case 'arriving':
        return { onCancel: back };
      default:
        return {
          onCancel: back,
          onWithoutFiles: online(home)
            ? undefined
            : () => send(move(ref, place, { withoutFiles: true })),
        };
    }
  };
  const dialog = drop.agent && drop.target && (
    <MoveMemberDialog
      key={n}
      agent={drop.agent}
      target={drop.target}
      open={open}
      onClose={() => setOpen(false)}
      onConfirm={(brain) => {
        setOpen(false);
        setArrived(drop.agent!.agent!.agent_id);
        send(move(drop.agent!.agent!, drop.target!.machine_id, { brain }));
      }}
    />
  );
  return { ...carry, busy: update.isPending, arrived, placeOf, moveOf, dialog };
}
