import './messages';
import { useState, type ReactNode } from 'react';
import { Badge, Board, Hang, VacantSlot, useSortable } from '../../scene';
import { Button, EmptyState } from '../../design';
import { useT } from '../../lib/i18n';
import { useAgents } from '../../lib/queries/agents';
import { useMachines } from '../../lib/queries/machines';
import { useHumanMembers, useWorkspace } from '../../lib/queries/workspaces';
import { readStorage, writeStorage } from '../../lib/storage';
import { MemberCard } from './MemberCard';
import { HumanCard, HumanBench } from './HumanCard';
import { MemberBench, type TopBarConfig } from './MemberBench';
import { broken } from './draft';

export type MembersPageProps = {
  workspaceId: string;
  page: { kind: string; id?: string };
  navigate: (page: { kind: 'members' } | { kind: 'member'; id: string }) => void;
  frame: (bar: TopBarConfig, body: ReactNode, board?: 'wall' | 'open') => ReactNode;
};

export function MembersPage({ workspaceId: workspace, page, navigate, frame }: MembersPageProps) {
  if (!workspace) return frame({}, null);
  if (page.kind === 'member' && page.id?.startsWith('human:'))
    return (
      <HumanBench
        key={`${workspace}/${page.id}`}
        workspace={workspace}
        id={page.id.slice(6)}
        onBack={() => navigate({ kind: 'members' })}
        frame={frame}
      />
    );
  if (page.kind === 'member' && page.id)
    return (
      <MemberBench
        key={`${workspace}/${page.id}`}
        workspace={workspace}
        id={page.id === 'new' ? null : page.id}
        onBack={() => navigate({ kind: 'members' })}
        onCreated={(id) => navigate({ kind: 'member', id })}
        frame={frame}
      />
    );
  return (
    <MemberBoard
      key={workspace}
      workspace={workspace}
      onOpen={(id) => navigate({ kind: 'member', id })}
      frame={frame}
    />
  );
}

const isIds = (v: unknown): v is string[] =>
  Array.isArray(v) && v.every((x) => typeof x === 'string');

function MemberBoard({
  workspace,
  onOpen,
  frame,
}: {
  workspace: string;
  onOpen: (id: string) => void;
  frame: MembersPageProps['frame'];
}) {
  const t = useT();
  const ref = { workspace_id: workspace };
  const agents = useAgents({ request_id: 'members', workspace: ref });
  const team = useWorkspace({ request_id: 'members', workspace: ref });
  const machines = useMachines({ request_id: 'members', workspace: ref });
  // There is no server-side order; each team keeps its own in this browser.
  const key = `members.order.${workspace}`;
  const [order, setOrder] = useState(() => readStorage(key, [], isIds));
  const rank = (id: string) => (order.includes(id) ? order.indexOf(id) : order.length);
  const list = (agents.data?.agents ?? [])
    .filter((a) => a.agent)
    .sort(
      (a, b) => rank(a.agent!.agent_id) - rank(b.agent!.agent_id) || a.created_at - b.created_at,
    );
  const ids = list.map((a) => a.agent!.agent_id);
  const { itemProps } = useSortable(ids, (next) => {
    setOrder(next);
    writeStorage(key, next);
  });
  const humans = useHumanMembers({ request_id: 'members', workspace: ref });
  const add = () => onOpen('new');

  const failed = [agents, team, machines, humans].filter(broken);
  if (failed.length)
    return frame(
      {},
      <EmptyState
        title={t('members.boardUnavailable')}
        action={
          <Button
            loading={failed.some((query) => query.isFetching)}
            onClick={() => failed.forEach((query) => void query.refetch())}
          >
            {t('members.retry')}
          </Button>
        }
      />,
    );
  if (agents.isPending)
    return frame(
      {},
      <Board layout="rails">
        {[0, 1, 2].map((i) => (
          <Hang key={i}>
            <Badge avatar={{}} name={null} size="card" ghost />
          </Hang>
        ))}
      </Board>,
      'wall',
    );
  if (!list.length && !humans.data?.length)
    return frame(
      {},
      <EmptyState
        title={t('members.empty')}
        action={
          <Button variant="primary" onClick={add} data-testid="add-member">
            {t('members.add')}
          </Button>
        }
      />,
    );
  return frame(
    {},
    <Board layout="rails">
      {humans.data?.map((person) => (
        <HumanCard
          key={person.person_id}
          workspace={workspace}
          person={person}
          onOpen={() => onOpen(`human:${person.person_id}`)}
        />
      ))}
      {list.map((agent) => (
        <MemberCard
          key={agent.agent!.agent_id}
          agent={agent}
          machine={machines.data?.machines.find((m) => m.machine_id === agent.machine_id)}
          workspace={team.data}
          handle={itemProps(agent.agent!.agent_id)}
          onOpen={() => onOpen(agent.agent!.agent_id)}
        />
      ))}
      <VacantSlot label={t('members.add')} onClick={add} shape="badge" data-testid="add-member" />
    </Board>,
    'wall',
  );
}
