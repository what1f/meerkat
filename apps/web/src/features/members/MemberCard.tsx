import type { ReactNode } from 'react';
import { BrandMark, modelBrand } from '../../design';
import { Badge, Hang, type SortableHandle } from '../../scene';
import { useT } from '../../lib/i18n';
import type * as DTO from '../../lib/api/generated';
import { useModelCatalog } from '../../lib/queries/catalogs';
import { agentStatus, useAgentAvatar } from '../../lib/queries/agents';
import { dataUrl } from './draft';

const brands = ['codex', 'claude_code', 'pi'] as const;
type Brand = (typeof brands)[number];
export const isBrand = (id: string): id is Brand => (brands as readonly string[]).includes(id);
export const modelLabel = (model: string) => model.split('/').pop() ?? model;

// How a member's model reads everywhere: the catalog's display name and the vendor logo,
// falling back to the raw id and the harness logo while the catalog has nothing to say.
export function useModelFace(
  workspace: string,
  machine_id: string,
  harness_id: string,
  model: string,
): { label: string; mark: ReactNode } {
  const t = useT();
  const catalog = useModelCatalog(
    { request_id: 'model-face', workspace: { workspace_id: workspace }, machine_id, harness_id },
    !!workspace && !!machine_id && !!harness_id,
  );
  const entry = catalog.data?.models.find((x) => x.model_id === model);
  const vendor = model ? modelBrand(model, entry?.display_name ?? '') : undefined;
  const brand = vendor ?? (isBrand(harness_id) ? harness_id : undefined);
  return {
    label: model ? entry?.display_name || modelLabel(model) : t('members.defaultModel'),
    mark: brand ? <BrandMark brand={brand} /> : null,
  };
}

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
  const avatar = useAgentAvatar({ request_id: 'members', agent: agent.agent }, !!agent.avatar_path);
  const { status } = agentStatus(agent, machine, workspace);
  const file = avatar.data?.avatar;
  const face = useModelFace(
    agent.agent?.workspace_id ?? '',
    agent.machine_id,
    agent.harness_id,
    agent.model,
  );
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
            {face.mark}
            {face.label}
          </>
        }
      />
    </Hang>
  );
}
