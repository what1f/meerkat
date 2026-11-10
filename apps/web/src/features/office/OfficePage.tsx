import { useState, type ReactNode } from 'react';
import type * as DTO from '../../lib/api/generated';
import { type Status, useToast } from '../../design';
import { AddRow, WorkRow } from '../../scene';
import { useMachines, useRenameMachine, useRevokeMachine } from '../../lib/queries/machines';
import { useAgents } from '../../lib/queries/agents';
import { useWorkspace } from '../../lib/queries/workspaces';
import { ApiError } from '../../lib/api/client';
import { useT } from '../../lib/i18n';
import { MemberSeat } from './MemberSeat';
import { MachineMenu, MachineTitle } from './MachineMenu';
import { BuiltinSignInDialog } from '../builtin-agent';
import { AddMachineDialog, usePairing } from './AddMachineDialog';
import { RemoveMachineDialog } from './RemoveMachineDialog';
import { LoadState } from './LoadState';
import { useMoves, type MoveDialog } from './useMoves';
import './messages';
import css from './OfficePage.module.css';

export type OfficePageProps = {
  workspaceId: string;
  navigate: (page: { kind: 'member'; id: string; from: 'office' }) => void;
  // Supplied by the app shell, which puts the top bar and body in place.
  frame: (bar: Record<string, never>, body: ReactNode) => ReactNode;
  moveDialog: MoveDialog;
};
const rowStatus: Partial<Record<DTO.ObservedState, Status>> = {
  running: 'ok',
  starting: 'busy',
  error: 'err',
};

export function OfficePage({
  workspaceId: workspace,
  navigate,
  frame,
  moveDialog,
}: OfficePageProps) {
  const t = useT();
  const toast = useToast();
  const ref = workspace ? { workspace_id: workspace } : null;
  const catalog = useMachines({ request_id: 'get_machine_catalog', workspace: null });
  const crew = useAgents({ request_id: 'list_agents', workspace: ref }, !!ref);
  const agents = crew.data?.agents;
  const team = useWorkspace({ request_id: 'get_workspace', workspace: ref }, !!ref).data;
  const rename = useRenameMachine();
  const revoke = useRevokeMachine();
  const pairing = usePairing();
  const [renaming, setRenaming] = useState<string | null>(null);
  // The machine stays named while the dialog animates out.
  const [removing, setRemoving] = useState<{ machine?: DTO.MachineCatalogItem; open: boolean }>({
    open: false,
  });
  const [signing, setSigning] = useState<{ id?: string; open: boolean }>({ open: false });
  const machines = (catalog.data?.machines ?? []).filter(
    (m) => m.registered && m.revoked_at === null,
  );
  const moves = useMoves(agents, machines, moveDialog);
  const bar = {};
  // Members that failed to load must not read as an empty office.
  const failed = [catalog, ...(ref ? [crew] : [])].filter((q) => q.isError && !q.data);
  if (failed.length || !catalog.data || (ref && !agents))
    return frame(
      bar,
      <div className={css.root}>
        <LoadState failed={failed} />
      </div>,
    );
  const signingMachine = (id?: string) => machines.find((m) => m.machine_id === id);
  const crewOf = (id: string) => (agents ?? []).filter((agent) => moves.placeOf(agent) === id);
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
            {...moves.rowProps(id)}
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
                onBuiltin={
                  machine.harnesses.some((h) => h.harness_id === 'builtin')
                    ? () => setSigning({ id, open: true })
                    : undefined
                }
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
                  arriving={moves.arrived === member}
                  // One move at a time, so a failed move always lands back where it started.
                  carry={moves.busy ? undefined : moves.seatHandle(member)}
                  onOpen={() => navigate({ kind: 'member', id: member, from: 'office' })}
                  move={moves.moveOf(agent)}
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
      {moves.dialog}
      <AddMachineDialog key={pairing.id} pairing={pairing} connected={fresh?.display_name} />
      {signing.open && signingMachine(signing.id) && (
        <BuiltinSignInDialog
          machine={signingMachine(signing.id)!}
          open={signing.open}
          onClose={() => setSigning({ ...signing, open: false })}
        />
      )}
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
