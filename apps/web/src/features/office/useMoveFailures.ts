// eslint-disable-next-line no-restricted-imports -- A move stops on the server; the office reacts once.
import { useEffect, useRef } from 'react';
import type * as DTO from '../../lib/api/generated';

/** Calls `onFailed` once for each member whose move stopped. */
export function useMoveFailures(
  agents: DTO.Agent[] | undefined,
  onFailed: (agent: DTO.Agent, ref: DTO.AgentRef) => void,
) {
  const reported = useRef(new Set<string>());
  // eslint-disable-next-line no-restricted-syntax -- The server reports the failure; the office answers it once.
  useEffect(() => {
    for (const agent of agents ?? []) {
      const ref = agent.agent;
      if (!ref || !agent.relocation?.failed || reported.current.has(ref.agent_id)) continue;
      reported.current.add(ref.agent_id);
      onFailed(agent, ref);
    }
  }, [agents, onFailed]);
}
