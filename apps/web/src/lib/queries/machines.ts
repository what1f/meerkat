import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import type * as DTO from '../api/generated';
import { call } from '../api/client';
import { keys } from './keys';
export function useMachines(input: DTO.CatalogRequest, enabled = true) {
  return useQuery({
    queryKey: [...keys.machines, 'get_machine_catalog', { ...input, request_id: undefined }],
    queryFn: ({ signal }) => call('get_machine_catalog', input, signal),
    enabled,
  });
}
// A code for one pairing session that renews itself when it expires.
// It sits outside the event keys so a machine changing state never swaps the command being copied.
export function usePairingCode(enabled: boolean, session = 0) {
  return useQuery({
    queryKey: ['pairing_code', session],
    queryFn: ({ signal }) =>
      call('create_pairing_code', { request_id: 'create_pairing_code', workspace: null }, signal),
    enabled,
    staleTime: Infinity,
    gcTime: 0,
    refetchInterval: (query) =>
      query.state.data ? Math.max(query.state.data.expires_at - Date.now(), 1000) : false,
  });
}
export function useRenameMachine() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (input: DTO.RenameMachineRequest) => call('rename_machine', input),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: keys.machines });
    },
  });
}
export function useRevokeMachine() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (input: DTO.RevokeMachineRequest) => call('revoke_machine', input),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: keys.machines });
    },
  });
}
export function useEnableRemoteAccess() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (_input: void) => call('enable_remote_access', null),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: keys.machines });
    },
  });
}
export function useRemoteAccess(enabled = true) {
  return useQuery({
    queryKey: [...keys.machines, 'get_remote_access', null],
    queryFn: ({ signal }) => call('get_remote_access', null, signal),
    enabled,
  });
}
export function useSetRemoteAccess() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (input: DTO.SetRemoteAccessRequest) => call('set_remote_access', input),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: keys.machines });
    },
  });
}
