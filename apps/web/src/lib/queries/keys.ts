import type { ManagementEventKind } from '../api/generated';
export const keys = {
  workspaces: ['workspaces'],
  agents: ['agents'],
  machines: ['machines'],
  catalogs: ['catalogs'],
  onboarding: ['onboarding'],
  figures: ['catalogs', 'figures'],
  humans: ['workspaces', 'humans'],
  tasks: ['workspaces', 'tasks'],
} as const;
const all = Object.values(keys);
export const eventKeys = {
  unspecified: all,
  workspace_created: [keys.workspaces],
  workspace_updated: [keys.workspaces, keys.agents, keys.onboarding],
  workspace_state_changed: [keys.workspaces, keys.agents],
  agent_created: [keys.agents, keys.onboarding],
  agent_updated: [keys.agents, keys.onboarding],
  provider_onboarding_changed: [keys.onboarding, keys.workspaces, keys.agents],
  catalog_changed: [keys.catalogs],
  agent_deleted: [keys.agents, keys.onboarding],
  machine_catalog_changed: [keys.machines, keys.catalogs, keys.agents],
} satisfies Record<ManagementEventKind, readonly (readonly string[])[]>;
