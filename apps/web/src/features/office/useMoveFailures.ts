// eslint-disable-next-line no-restricted-imports -- A move stops on the server; the office reacts once.
import { useEffect, useRef } from 'react';
import type * as DTO from '../../lib/api/generated';

type Stop = Extract<DTO.AgentRelocationState, 'failed' | 'stalled'>;

/** Calls `onStopped` once each time a member's move fails or they cannot start where they moved. */
export function useMoveFailures(
  agents: DTO.Agent[] | undefined,
  onStopped: (agent: DTO.Agent, ref: DTO.AgentRef, state: Stop) => void,
) {
  const reported = useRef(new Map<string, Stop>());
  // eslint-disable-next-line no-restricted-syntax -- The server reports the failure; the office answers it once.
  useEffect(() => {
    for (const agent of agents ?? []) {
      const ref = agent.agent;
      if (!ref) continue;
      const state = agent.relocation?.state;
      if (state !== 'failed' && state !== 'stalled') {
        reported.current.delete(ref.agent_id);
        continue;
      }
      if (reported.current.get(ref.agent_id) === state) continue;
      reported.current.set(ref.agent_id, state);
      onStopped(agent, ref, state);
    }
  }, [agents, onStopped]);
}
