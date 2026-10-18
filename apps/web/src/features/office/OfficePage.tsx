import { useState, type ReactNode } from 'react';
import type * as DTO from '../../lib/api/generated';
import { type Status, useToast } from '../../design';
import { AddRow, WorkRow, useCarry } from '../../scene';
import { useMachines, useRenameMachine, useRevokeMachine } from '../../lib/queries/machines';
import { useAgents, useUpdateAgent } from '../../lib/queries/agents';
import { useWorkspace } from '../../lib/queries/workspaces';
import { ApiError } from '../../lib/api/client';
import { useT } from '../../lib/i18n';
import { MemberSeat } from './MemberSeat';
import { MachineMenu, MachineTitle } from './MachineMenu';
import { AddMachineDialog, usePairing } from './AddMachineDialog';
import { RemoveMachineDialog } from './RemoveMachineDialog';
import { LoadState } from './LoadState';
import './messages';
import css from './OfficePage.module.css';

export type OfficePageProps = {
  workspaceId: string;
  navigate: (page: { kind: 'member'; id: string }) => void;
  // Supplied by the app shell, which puts the top bar and body in place.
  frame: (bar: { title: string }, body: ReactNode) => ReactNode;
};
const rowStatus: Partial<Record<DTO.ObservedState, Status>> = {
  running: 'ok',
  starting: 'busy',
  error: 'err',
};
const move = (agent: DTO.AgentRef, machine_id: string): DTO.UpdateAgentRequest => ({
  request_id: 'move_agent',
  agent,
  machine_id,
  figure: null,
  launch_arguments: null,
  name: null,
  role: null,
  bio: null,
  harness_id: null,
  model: null,
  thinking_level: null,
  auto_reaction: null,
  soul: null,
  cli_path: null,
  environment_variables: null,
  avatar: null,
  attention: null,
  provider_agent_config_json: null,
  provider_agent_secrets_json: null,
});

export function OfficePage({ workspaceId: workspace, navigate, frame }: OfficePageProps) {
  const t = useT();
  const toast = useToast();
  const ref = workspace ? { workspace_id: workspace } : null;
  const catalog = useMachines({ request_id: 'get_machine_catalog', workspace: null });
  const crew = useAgents({ request_id: 'list_agents', workspace: ref }, !!ref);
  const agents = crew.data?.agents;
  const team = useWorkspace({ request_id: 'get_workspace', workspace: ref }, !!ref).data;
  const update = useUpdateAgent();
  const rename = useRenameMachine();
  const revoke = useRevokeMachine();
  const pairing = usePairing();
  const [renaming, setRenaming] = useState<string | null>(null);
  // The machine stays named while the dialog animates out.
  const [removing, setRemoving] = useState<{ machine?: DTO.MachineCatalogItem; open: boolean }>({
    open: false,
  });
  const [arrived, setArrived] = useState<string | null>(null);
  const { seatHandle, rowProps } = useCarry((id, machine) => {
    if (!workspace) return;
    setArrived(id);
    update.mutate(move({ workspace_id: workspace, agent_id: id }, machine), {
      onError: () => toast({ message: t('office.moveFailed') }),
    });
  });
  const bar = { title: t('common.office') };
  // Members that failed to load must not read as an empty office.
  const failed = [catalog, ...(ref ? [crew] : [])].filter((q) => q.isError && !q.data);
  if (failed.length || !catalog.data || (ref && !agents))
    return frame(
      bar,
      <div className={css.root}>
        <LoadState failed={failed} />
      </div>,
    );
  const machines = catalog.data.machines.filter((m) => m.registered && m.revoked_at === null);
  // Until the catalog refetches, a member who was just carried stays where they were dropped.
  const moving = update.isPending ? update.variables : undefined;
  const placeOf = (agent: DTO.Agent) =>
    moving && moving.agent?.agent_id === agent.agent?.agent_id
      ? moving.machine_id
      : agent.machine_id;
  const crewOf = (id: string) => (agents ?? []).filter((agent) => placeOf(agent) === id);
  const fresh = pairing.known && machines.find((m) => !pairing.known!.has(m.machine_id));
  const nameOf = (m: DTO.MachineCatalogItem) =>
    rename.isPending && rename.variables.machine_id === m.machine_id
      ? rename.variables.name
      : m.display_name;
  return frame(
    bar,
    <div className={css.root}>
      {machines.map((machine) => {
        const id = machine.machine_id;
        return (
          <WorkRow
            key={id}
            {...rowProps(id)}
            data-testid={`machine-row-${id}`}
            status={rowStatus[machine.observed_state] ?? 'idle'}
            arriving={!!pairing.known && !pairing.known.has(id)}
            title={
              <MachineTitle
                name={nameOf(machine)}
                renaming={renaming === id}
                onDone={() => setRenaming(null)}
                onCommit={(name) =>
                  rename.mutate(
                    { request_id: 'rename_machine', machine_id: id, name },
                    { onError: () => toast({ message: t('office.renameFailed') }) },
                  )
                }
              />
            }
            menu={
              <MachineMenu
                id={id}
                removable={!machine.is_local}
                onRename={() => setRenaming(id)}
                onRemove={() =>
                  // The Runtime keeps a computer while members are assigned to it.
                  crewOf(id).length
                    ? toast({ message: t('office.moveCrewFirst') })
                    : setRemoving({ machine, open: true })
                }
              />
            }
          >
            {crewOf(id).map((agent) => {
              const member = agent.agent?.agent_id ?? '';
              return (
                <MemberSeat
                  key={member}
                  agent={agent}
                  machine={machine}
                  workspace={team}
                  arriving={arrived === member}
                  // One move at a time, so a failed move always lands back where it started.
                  carry={update.isPending ? undefined : seatHandle(member)}
                  onOpen={() => navigate({ kind: 'member', id: member })}
                />
              );
            })}
          </WorkRow>
        );
      })}
      <AddRow
        data-testid="add-machine"
        label={t('office.add')}
        onClick={() => void pairing.start(machines.map((m) => m.machine_id))}
      />
      <AddMachineDialog key={pairing.id} pairing={pairing} connected={fresh?.display_name} />
      <RemoveMachineDialog
        name={removing.machine?.display_name ?? ''}
        open={removing.open}
        onClose={() => setRemoving({ ...removing, open: false })}
        onConfirm={() => {
          if (!removing.machine) return;
          revoke.mutate(
            { request_id: 'revoke_machine', machine_id: removing.machine.machine_id },
            {
              onError: (error) =>
                toast({
                  message:
                    error instanceof ApiError && error.detail === 'machine_has_assigned_agents'
                      ? t('office.moveCrewFirst')
                      : t('office.removeFailed'),
                }),
            },
          );
          setRemoving({ ...removing, open: false });
        }}
      />
    </div>,
  );
}
