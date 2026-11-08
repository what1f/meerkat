import type * as DTO from '../../lib/api/generated';
import { Icon, Menu } from '../../design';
import { Seat, type CarryHandle } from '../../scene';
import { useT } from '../../lib/i18n';
import './messages';
import { agentStatus, useAgentAvatar } from '../../lib/queries/agents';
import { useFigurePacks } from '../../lib/queries/catalogs';
import { figureId, packFrames } from '../../lib/assets';
import { dataUrl } from '../../lib/assets';

// A stable pick per member, so the same desk and figure come back on every visit.
const pick = (id: string, n: number) =>
  [...id].reduce((h, c) => (h * 31 + c.charCodeAt(0)) >>> 0, 7) % n;

export function MemberSeat({
  agent,
  machine,
  workspace,
  arriving,
  carry,
  onOpen,
  move,
}: {
  agent: DTO.Agent;
  machine: DTO.MachineCatalogItem;
  workspace: DTO.Workspace | undefined;
  arriving: boolean;
  carry?: CarryHandle;
  onOpen: () => void;
  // Set while the member is on their way here from another computer.
  move?: { onCancel: () => void; onWithoutFiles?: () => void };
}) {
  const t = useT();
  const id = agent.agent?.agent_id ?? '';
  const { screen } = agentStatus(agent, machine, workspace);
  const workspaceId = agent.agent?.workspace_id ?? '';
  const packs = useFigurePacks({ workspace: { workspace_id: workspaceId } }, !!workspaceId);
  const pack = packs.data?.find((p) => p.id === agent.figure);
  // The default look stands in as a built-in figure chosen from the member id.
  // Off shift, nobody is at their desk.
  const away = workspace?.desired_state === 'paused';
  const avatar = useAgentAvatar({ request_id: 'seat', agent: agent.agent }, !!agent.avatar_path);
  const file = avatar.data?.avatar;
  return (
    <Seat
      setup={pick(id, 2) ? 'mini' : 'laptop'}
      screen={screen}
      occupant={
        away
          ? 'empty-chair'
          : {
              figure: figureId(agent.figure),
              pack: pack && packFrames(pack, workspaceId),
              seed: id,
            }
      }
      arriving={arriving}
      moving={!!move}
      wrapBox={
        move &&
        ((box) => (
          <span>
            <Menu trigger={box}>
              <Menu.Item leading={<Icon name="x" size={14} />} onSelect={move.onCancel}>
                {t('office.cancelMove')}
              </Menu.Item>
              {move.onWithoutFiles && (
                <Menu.Item leading={<Icon name="box" size={14} />} onSelect={move.onWithoutFiles}>
                  {t('office.moveWithoutFiles')}
                </Menu.Item>
              )}
            </Menu>
          </span>
        ))
      }
      label={agent.name}
      badge={{ name: agent.name, avatar: file ? dataUrl(file) : undefined }}
      onClick={move ? undefined : onOpen}
      carry={carry}
    />
  );
}
