import { useQuery, useMutation, useQueryClient, useIsMutating } from '@tanstack/react-query';
import type * as DTO from '../api/generated';
import { call } from '../api/client';
import { keys } from './keys';
import { usePoll } from '../usePoll';
export function useProviderOnboarding(input: DTO.GetProviderOnboardingRequest, enabled = true) {
  return useQuery({
    queryKey: [...keys.onboarding, 'get_provider_onboarding', { ...input, request_id: undefined }],
    queryFn: ({ signal }) => call('get_provider_onboarding', input, signal),
    enabled,
    refetchInterval: usePoll(),
  });
}
export function useAdvanceProviderOnboarding() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (input: DTO.AdvanceProviderOnboardingRequest) =>
      call('advance_provider_onboarding', input),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: keys.onboarding });
    },
  });
}
export function useAgentOnboarding(input: DTO.GetAgentOnboardingRequest, enabled = true) {
  return useQuery({
    queryKey: [...keys.onboarding, 'get_agent_onboarding', { ...input, request_id: undefined }],
    queryFn: ({ signal }) => call('get_agent_onboarding', input, signal),
    enabled,
    refetchInterval: usePoll(),
  });
}
export function useAdvanceAgentOnboarding() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (input: DTO.AdvanceAgentOnboardingRequest) =>
      call('advance_agent_onboarding', input),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: keys.onboarding });
    },
  });
}

type FeishuAccountSwitchInput = { action: 'switch_start' | 'switch_poll' | 'switch_cancel' };
export type FeishuAccountSwitch = {
  state: 'required' | 'ready' | 'waiting' | 'scanned';
  qr_code?: string;
  poll_interval_seconds?: number;
};
export function useFeishuAccountSwitch(input: DTO.GetWorkspaceRequest, enabled = true) {
  const switching = useIsMutating({
    mutationKey: [...keys.onboarding, 'feishu_account_switch'],
    predicate: (mutation) => {
      const variables = mutation.state.variables as DTO.GetWorkspaceRequest | undefined;
      return variables?.workspace?.workspace_id === input.workspace?.workspace_id;
    },
  });
  return useQuery({
    queryKey: [...keys.onboarding, 'feishu_account_switch', { ...input, request_id: undefined }],
    queryFn: async ({ signal }) => {
      const response = await call(
        'invoke_provider_interface',
        {
          ...input,
          interface_id: 'feishu.workspace.console_login',
          input_json: JSON.stringify({ action: 'switch_poll' } satisfies FeishuAccountSwitchInput),
        },
        signal,
      );
      return JSON.parse(response.output_json) as FeishuAccountSwitch;
    },
    enabled: enabled && switching === 0,
    refetchInterval: usePoll(),
  });
}
export function useStartFeishuAccountSwitch() {
  const client = useQueryClient();
  return useMutation({
    mutationKey: [...keys.onboarding, 'feishu_account_switch'],
    mutationFn: async (input: DTO.GetWorkspaceRequest) => {
      await client.cancelQueries({
        queryKey: [
          ...keys.onboarding,
          'feishu_account_switch',
          { ...input, request_id: undefined },
        ],
      });
      const response = await call('invoke_provider_interface', {
        ...input,
        interface_id: 'feishu.workspace.console_login',
        input_json: JSON.stringify({ action: 'switch_start' } satisfies FeishuAccountSwitchInput),
      });
      return JSON.parse(response.output_json) as FeishuAccountSwitch;
    },
    onSuccess: async (state, input) => {
      const queryKey = [
        ...keys.onboarding,
        'feishu_account_switch',
        { ...input, request_id: undefined },
      ];
      await client.cancelQueries({ queryKey });
      client.setQueryData(queryKey, state);
    },
  });
}
export function useCancelFeishuAccountSwitch() {
  const client = useQueryClient();
  return useMutation({
    mutationKey: [...keys.onboarding, 'feishu_account_switch'],
    mutationFn: async (input: DTO.GetWorkspaceRequest) => {
      await client.cancelQueries({
        queryKey: [
          ...keys.onboarding,
          'feishu_account_switch',
          { ...input, request_id: undefined },
        ],
      });
      const response = await call('invoke_provider_interface', {
        ...input,
        interface_id: 'feishu.workspace.console_login',
        input_json: JSON.stringify({ action: 'switch_cancel' } satisfies FeishuAccountSwitchInput),
      });
      return JSON.parse(response.output_json) as FeishuAccountSwitch;
    },
    onSuccess: async (state, input) => {
      const queryKey = [
        ...keys.onboarding,
        'feishu_account_switch',
        { ...input, request_id: undefined },
      ];
      await client.cancelQueries({ queryKey });
      client.setQueryData(queryKey, state);
    },
  });
}
