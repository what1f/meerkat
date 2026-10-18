import './messages';
import { useState, type ReactNode } from 'react';
import { Badge, Board, Hang, VacantSlot, useSortable } from '../../scene';
import { Button, EmptyState } from '../../design';
import { useT } from '../../lib/i18n';
import { useAgents } from '../../lib/queries/agents';
import { useMachines } from '../../lib/queries/machines';
import { useWorkspace } from '../../lib/queries/workspaces';
import { readStorage, writeStorage } from '../../lib/storage';
import { MemberCard } from './MemberCard';
import { LeadCard } from './LeadCard';
import { MemberBench, type TopBarConfig } from './MemberBench';
import { broken } from './draft';

export type MembersPageProps = {
  workspaceId: string;
  page: { kind: string; id?: string };
  navigate: (page: { kind: 'members' } | { kind: 'member'; id: string }) => void;
  frame: (bar: TopBarConfig, body: ReactNode) => ReactNode;
};

export function MembersPage({ workspaceId: workspace, page, navigate, frame }: MembersPageProps) {
  if (!workspace) return frame({}, null);
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
  return frame(
    {},
    <MemberBoard
      key={workspace}
      workspace={workspace}
      onOpen={(id) => navigate({ kind: 'member', id })}
    />,
  );
}

const isIds = (v: unknown): v is string[] =>
  Array.isArray(v) && v.every((x) => typeof x === 'string');

function MemberBoard({ workspace, onOpen }: { workspace: string; onOpen: (id: string) => void }) {
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
  const provider = team.data?.provider_id;
  const lead = provider === 'feishu' || provider === 'telegram' ? provider : null;
  const add = () => onOpen('new');

  const failed = [agents, team, machines].filter(broken);
  if (failed.length)
    return (
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
      />
    );
  if (agents.isPending)
    return (
      <Board layout="rails">
        {[0, 1, 2].map((i) => (
          <Hang key={i}>
            <Badge avatar={{}} name={null} size="card" ghost />
          </Hang>
        ))}
      </Board>
    );
  if (!list.length && !lead)
    return (
      <EmptyState
        title={t('members.empty')}
        action={
          <Button variant="primary" onClick={add} data-testid="add-member">
            {t('members.add')}
          </Button>
        }
      />
    );
  return (
    <Board layout="rails">
      {lead && <LeadCard workspace={workspace} provider={lead} />}
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
    </Board>
  );
}
