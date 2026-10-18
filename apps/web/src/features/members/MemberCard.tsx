import { BrandMark } from '../../design';
import { Badge, Hang, type SortableHandle } from '../../scene';
import { useT } from '../../lib/i18n';
import type * as DTO from '../../lib/api/generated';
import { agentStatus, useAgentAvatar } from '../../lib/queries/agents';
import { dataUrl } from './draft';

const brands = ['codex', 'claude_code', 'pi'] as const;
type Brand = (typeof brands)[number];
export const isBrand = (id: string): id is Brand => (brands as readonly string[]).includes(id);
export const modelLabel = (model: string) => model.split('/').pop() ?? model;

export function MemberCard({
  agent,
  machine,
  workspace,
  handle,
  onOpen,
}: {
  agent: DTO.Agent;
  machine: DTO.MachineCatalogItem | undefined;
  workspace: DTO.Workspace | undefined;
  handle: SortableHandle;
  onOpen: () => void;
}) {
  const t = useT();
  const avatar = useAgentAvatar({ request_id: 'members', agent: agent.agent }, !!agent.avatar_path);
  const { status } = agentStatus(agent, machine, workspace);
  const file = avatar.data?.avatar;
  return (
    <Hang
      data-testid={`member-card-${agent.agent?.agent_id}`}
      swingOnHover
      onClick={onOpen}
      {...handle}
    >
      <Badge
        size="card"
        avatar={{
          src: file ? dataUrl(file) : undefined,
          status,
          statusTip: agent.pause_reason ?? undefined,
        }}
        name={agent.name}
        role={agent.role}
        bio={agent.bio}
        dimmed={!!agent.pause_reason}
        foot={
          <>
            {isBrand(agent.harness_id) && <BrandMark brand={agent.harness_id} />}
            {agent.model ? modelLabel(agent.model) : t('members.defaultModel')}
          </>
        }
      />
    </Hang>
  );
}
