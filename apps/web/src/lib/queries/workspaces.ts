import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import type * as DTO from '../api/generated';
import { ApiError, call } from '../api/client';
import { keys } from './keys';
export function useWorkspaces(input: DTO.ListWorkspacesRequest, enabled = true) {
  return useQuery({
    queryKey: [...keys.workspaces, 'list_workspaces', { ...input, request_id: undefined }],
    queryFn: ({ signal }) => call('list_workspaces', input, signal),
    enabled,
  });
}
export function useWorkspace(input: DTO.GetWorkspaceRequest, enabled = true) {
  return useQuery({
    queryKey: [...keys.workspaces, 'get_workspace', { ...input, request_id: undefined }],
    queryFn: ({ signal }) => call('get_workspace', input, signal),
    enabled,
  });
}
export function useWorkspaceImage(input: DTO.WorkspaceImageRequest, enabled = true) {
  return useQuery({
    queryKey: [...keys.workspaces, 'get_workspace_image', { ...input, request_id: undefined }],
    queryFn: ({ signal }) => call('get_workspace_image', input, signal),
    enabled,
  });
}
export function useUploadWorkspaceImage() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (input: DTO.UploadWorkspaceImageRequest) => call('upload_workspace_image', input),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: keys.workspaces });
    },
  });
}
export function useCreateWorkspace() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (input: DTO.CreateWorkspaceRequest) => call('create_workspace', input),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: keys.workspaces });
    },
  });
}
// Writes that share a scope run one after another, so the last one made is the one kept.
export function useUpdateWorkspace(scope?: string) {
  const client = useQueryClient();
  return useMutation({
    scope: scope ? { id: scope } : undefined,
    mutationFn: (input: DTO.UpdateWorkspaceRequest) => call('update_workspace', input),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: keys.workspaces });
    },
  });
}
export function useSetWorkspaceState(scope?: string) {
  const client = useQueryClient();
  return useMutation({
    scope: scope ? { id: scope } : undefined,
    mutationFn: (input: DTO.SetWorkspaceStateRequest) => call('set_workspace_state', input),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: keys.workspaces });
    },
  });
}

export function useHumanMembers(input: DTO.GetWorkspaceRequest) {
  return useQuery({
    queryKey: [...keys.humans, { ...input, request_id: undefined }],
    queryFn: ({ signal }) => call('list_human_members', input, signal),
  });
}
export function useSyncHumanProfile() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (input: DTO.SyncHumanProfileRequest) => call('sync_human_profile', input),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: keys.humans });
    },
  });
}
export function useUpdateHumanPosition() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (input: DTO.UpdateHumanPositionRequest) => call('update_human_position', input),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: keys.humans });
    },
  });
}

type WorkspaceTasksInput = DTO.WorkspaceTasksInput;
export type WorkspaceTask = {
  id: string;
  text: string;
  completed: boolean;
  assignee?: { agent_id?: string; name?: string; avatar?: DTO.ResourceRef };
};
export type WorkspaceTasks = { enabled: boolean; name?: string; items: WorkspaceTask[] };
export function useWorkspaceTasks(input: DTO.GetWorkspaceRequest, enabled = true) {
  return useQuery({
    queryKey: [...keys.tasks, { ...input, request_id: undefined }],
    retry: (count, error) =>
      count < 3 && !(error instanceof ApiError && error.code === 'invalid_argument'),
    queryFn: async ({ signal }) => {
      const response = await call(
        'invoke_provider_interface',
        {
          ...input,
          interface_id: 'feishu.workspace.tasks',
          input_json: JSON.stringify({ action: 'list' } satisfies WorkspaceTasksInput),
        },
        signal,
      );
      return JSON.parse(response.output_json) as WorkspaceTasks;
    },
    enabled,
  });
}

export function useRenameWorkspaceTasklist() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (input: DTO.GetWorkspaceRequest & { name: string }) =>
      call('invoke_provider_interface', {
        request_id: input.request_id,
        workspace: input.workspace,
        interface_id: 'feishu.workspace.tasks',
        input_json: JSON.stringify({
          action: 'rename',
          name: input.name,
        } satisfies WorkspaceTasksInput),
      }),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: keys.tasks });
    },
  });
}

// Timers the team's members set to wake themselves, soonest first.
export function useTimers(input: DTO.GetWorkspaceRequest, enabled = true) {
  return useQuery({
    queryKey: [...keys.timers, { ...input, request_id: undefined }],
    queryFn: ({ signal }) => call('list_timers', input, signal),
    enabled,
  });
}
export function useCancelTimer() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (input: DTO.TimerRequest) => call('cancel_timer', input),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: keys.timers });
    },
  });
}
export function useRestoreTimer() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (input: DTO.TimerRequest) => call('restore_timer', input),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: keys.timers });
    },
  });
}
