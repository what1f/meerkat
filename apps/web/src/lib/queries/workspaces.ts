import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import type * as DTO from '../api/generated';
import { call } from '../api/client';
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
export function useUpdateWorkspace() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (input: DTO.UpdateWorkspaceRequest) => call('update_workspace', input),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: keys.workspaces });
    },
  });
}
export function useSetWorkspaceState() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (input: DTO.SetWorkspaceStateRequest) => call('set_workspace_state', input),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: keys.workspaces });
    },
  });
}
