import type * as DTO from '../../lib/api/generated';

export type Draft = {
  name: string;
  role: string;
  bio: string;
  soul: string;
  figure: string;
  auto_reaction: string;
  machine_id: string;
  harness_id: string;
  model: string;
  thinking_level: string;
  cli_path: string;
  environment_variables: DTO.EnvironmentVariable[];
  launch_arguments: string[];
};
export type AvatarUpload = DTO.AgentAvatarUpload;
export const blank: Draft = {
  name: '',
  role: '',
  bio: '',
  soul: '',
  figure: 'default',
  auto_reaction: '',
  machine_id: '',
  harness_id: '',
  model: '',
  thinking_level: '',
  cli_path: '',
  environment_variables: [],
  launch_arguments: [],
};
export const fields = Object.keys(blank) as (keyof Draft)[];
export const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
// Member ids are internal; they only need to satisfy the Runtime identifier rule.
export const newId = () => `m${crypto.randomUUID().replaceAll('-', '').slice(0, 12)}`;
export { dataUrl } from '../../lib/assets';
// A read counts as failed once its retries are used up, even while it is tried again.
export const broken = (query: { isError: boolean; data: unknown; errorUpdateCount: number }) =>
  query.isError || (!query.data && query.errorUpdateCount > 0);

export function pick(agent: DTO.Agent): Draft {
  return Object.fromEntries(fields.map((key) => [key, agent[key]])) as Draft;
}
export const emptyUpdate = (): Omit<DTO.UpdateAgentRequest, 'agent'> => ({
  request_id: crypto.randomUUID(),
  relocate_without_files: null,
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
  machine_id: null,
  avatar: null,
  attention: null,
  provider_agent_config_json: null,
  provider_agent_secrets_json: null,
});

// Writes to one member go out one at a time; a call made meanwhile does nothing.
export async function oneAtATime<T>(busy: { current: boolean }, write: () => Promise<T>) {
  if (busy.current) return null;
  busy.current = true;
  try {
    return await write();
  } finally {
    busy.current = false;
  }
}
