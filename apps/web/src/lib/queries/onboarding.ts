import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
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
