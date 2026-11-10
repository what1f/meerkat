import type * as DTO from '../../lib/api/generated';

export type Brain = Pick<DTO.UpdateAgentRequest, 'harness_id' | 'model' | 'thinking_level'>;

const request = (agent: DTO.AgentRef): DTO.UpdateAgentRequest => ({
  request_id: 'move_agent',
  agent,
  machine_id: null,
  relocate_without_files: null,
  retry_arrival: null,
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

/** Sends a member to another computer with the AI it runs there; back to where they came from calls the move off. */
export const move = (
  agent: DTO.AgentRef,
  machine_id: string,
  options: { withoutFiles?: boolean; brain?: Brain } = {},
): DTO.UpdateAgentRequest => ({
  ...request(agent),
  ...options.brain,
  machine_id,
  relocate_without_files: options.withoutFiles || null,
});

/** Wakes a member who has not started on their new computer once more. */
export const retryArrival = (agent: DTO.AgentRef): DTO.UpdateAgentRequest => ({
  ...request(agent),
  retry_arrival: true,
});
