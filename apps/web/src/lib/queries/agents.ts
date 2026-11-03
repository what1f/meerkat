import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import type * as DTO from '../api/generated';
import { call } from '../api/client';
import { keys } from './keys';
export function useAgents(input: DTO.ListAgentsRequest, enabled = true) {
  return useQuery({
    queryKey: [...keys.agents, 'list_agents', { ...input, request_id: undefined }],
    queryFn: ({ signal }) => call('list_agents', input, signal),
    enabled,
  });
}
export function useAgent(input: DTO.GetAgentRequest, enabled = true) {
  return useQuery({
    queryKey: [...keys.agents, 'get_agent', { ...input, request_id: undefined }],
    queryFn: ({ signal }) => call('get_agent', input, signal),
    enabled,
  });
}
export function useAgentAvatar(input: DTO.GetAgentAvatarRequest, enabled = true) {
  return useQuery({
    queryKey: [...keys.agents, 'get_agent_avatar', { ...input, request_id: undefined }],
    queryFn: ({ signal }) => call('get_agent_avatar', input, signal),
    enabled,
  });
}
export function useExecutionDetails(input: DTO.GetExecutionDetailsRequest, enabled = true) {
  return useQuery({
    queryKey: [...keys.agents, 'get_execution_details', { ...input, request_id: undefined }],
    queryFn: ({ signal }) => call('get_execution_details', input, signal),
    enabled,
  });
}
export function useAgentFirstReply(input: DTO.GetAgentRequest, enabled = true) {
  return useQuery({
    queryKey: [...keys.agents, 'get_agent_first_reply', { ...input, request_id: undefined }],
    queryFn: ({ signal }) => call('get_agent_first_reply', input, signal),
    enabled,
  });
}
export function useCancelAttempt() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (input: DTO.CancelAttemptRequest) => call('cancel_attempt', input),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: keys.agents });
    },
  });
}
export function useCreateAgent() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (input: DTO.CreateAgentRequest) => call('create_agent', input),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: keys.agents });
    },
  });
}
export function useUpdateAgent() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (input: DTO.UpdateAgentRequest) => call('update_agent', input),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: keys.agents });
    },
  });
}
export function useResumeAgent() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (input: DTO.GetAgentRequest) => call('resume_agent', input),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: keys.agents });
    },
  });
}
export function useDeleteAgent() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (input: DTO.DeleteAgentRequest) => call('delete_agent', input),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: keys.agents });
    },
  });
}

export function agentStatus(
  agent: DTO.Agent,
  machine: DTO.MachineCatalogItem | undefined,
  workspace: DTO.Workspace | undefined,
): {
  status: 'ok' | 'busy' | 'idle' | 'err';
  screen: 'dark' | 'asleep' | 'boot' | 'alert' | 'busy' | 'ok';
} {
  // Off duty nobody replies, so no member needs attention and every screen is off.
  if (workspace?.desired_state === 'paused') return { status: 'idle', screen: 'dark' };
  // On an offline computer everyone sleeps, even a member who needs attention.
  if (
    !machine ||
    !machine.registered ||
    machine.revoked_at !== null ||
    machine.observed_state === 'stopped'
  )
    return { status: 'idle', screen: 'dark' };
  if (
    agent.pause_reason ||
    workspace?.observed_state === 'error' ||
    machine.observed_state === 'error'
  )
    return { status: 'err', screen: 'alert' };
  if (workspace?.observed_state === 'paused' || machine.observed_state === 'paused')
    return { status: 'idle', screen: 'asleep' };
  if (workspace?.observed_state === 'starting' || machine.observed_state === 'starting')
    return { status: 'busy', screen: 'boot' };
  if (workspace?.observed_state !== 'running' || machine.observed_state !== 'running')
    return { status: 'idle', screen: 'dark' };
  if (agent.activity === 'busy') return { status: 'busy', screen: 'busy' };
  return { status: 'ok', screen: 'ok' };
}
