import type * as DTO from '../../lib/api/generated';
import { Seat, type CarryHandle } from '../../scene';
import { figures } from '../../scene/figures';
import { agentStatus } from '../../lib/queries/agents';

const builtIn = Object.keys(figures);
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
}: {
  agent: DTO.Agent;
  machine: DTO.MachineCatalogItem;
  workspace: DTO.Workspace | undefined;
  arriving: boolean;
  carry?: CarryHandle;
  onOpen: () => void;
}) {
  const id = agent.agent?.agent_id ?? '';
  const { screen } = agentStatus(agent, machine, workspace);
  // Figure packs and the default have no seated frames; they sit as a built-in figure.
  const figure = figures[agent.figure] ? agent.figure : builtIn[pick(id, builtIn.length)]!;
  // Off shift, nobody is at their desk.
  const away = workspace?.desired_state === 'paused';
  return (
    <Seat
      setup={pick(id, 2) ? 'mini' : 'laptop'}
      screen={screen}
      occupant={away ? 'empty-chair' : { figure }}
      arriving={arriving}
      label={agent.name}
      onClick={onOpen}
      carry={carry}
    />
  );
}
