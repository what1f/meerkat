import type * as DTO from '../../lib/api/generated';

/** Sends a member to another computer; back to where they are calls the move off. */
export const move = (
  agent: DTO.AgentRef,
  machine_id: string,
  withoutFiles = false,
): DTO.UpdateAgentRequest => ({
  request_id: 'move_agent',
  agent,
  machine_id,
  relocate_without_files: withoutFiles || null,
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
