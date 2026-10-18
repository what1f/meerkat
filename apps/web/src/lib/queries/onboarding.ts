import {
  useQuery,
  useQueries,
  useMutation,
  useQueryClient,
  useIsMutating,
} from '@tanstack/react-query';
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
// Both flows share console sign-in; `start` always shows a new code.
type ConsoleLoginAction = 'start' | 'poll' | 'switch_start' | 'switch_poll' | 'switch_cancel';
type ConsoleLoginFlow = 'feishu_account_switch' | 'feishu_console_login';
export type FeishuAccountSwitch = {
  state: 'required' | 'ready' | 'waiting' | 'scanned';
  qr_code?: string;
  poll_interval_seconds?: number;
};
const objectKey = (name: string, input: object) => [
  ...keys.onboarding,
  name,
  { ...input, request_id: undefined },
];
async function consoleLogin(
  input: DTO.GetWorkspaceRequest,
  action: ConsoleLoginAction,
  signal?: AbortSignal,
) {
  const response = await call(
    'invoke_provider_interface',
    {
      ...input,
      interface_id: 'feishu.workspace.console_login',
      input_json: JSON.stringify({ action } satisfies { action: ConsoleLoginAction }),
    },
    signal,
  );
  return JSON.parse(response.output_json) as FeishuAccountSwitch;
}
function useConsoleLogin(
  flow: ConsoleLoginFlow,
  action: ConsoleLoginAction,
  input: DTO.GetWorkspaceRequest,
  enabled: boolean,
) {
  // Pending start/cancel calls own the cache; a late poll must not overwrite them.
  const pending = useIsMutating({
    mutationKey: [...keys.onboarding, flow],
    predicate: (mutation) => {
      const variables = mutation.state.variables as DTO.GetWorkspaceRequest | undefined;
      return variables?.workspace?.workspace_id === input.workspace?.workspace_id;
    },
  });
  const poll = usePoll();
  const stopOnError = flow === 'feishu_account_switch';
  return useQuery({
    queryKey: objectKey(flow, input),
    queryFn: ({ signal }) => consoleLogin(input, action, signal),
    enabled: (q) => enabled && pending === 0 && !(stopOnError && q.state.error),
    retry: stopOnError ? false : undefined,
    refetchOnWindowFocus: stopOnError ? false : undefined,
    refetchOnReconnect: stopOnError ? false : undefined,
    refetchInterval: (q) => (stopOnError && q.state.error ? false : poll(q)),
  });
}
function useConsoleLoginAction(flow: ConsoleLoginFlow, action: ConsoleLoginAction) {
  const client = useQueryClient();
  return useMutation({
    mutationKey: [...keys.onboarding, flow],
    mutationFn: async (input: DTO.GetWorkspaceRequest) => {
      await client.cancelQueries({ queryKey: objectKey(flow, input) });
      return consoleLogin(input, action);
    },
    onSuccess: async (state, input) => {
      await client.cancelQueries({ queryKey: objectKey(flow, input) });
      client.setQueryData(objectKey(flow, input), state);
    },
  });
}
export function useFeishuAccountSwitch(input: DTO.GetWorkspaceRequest, enabled = true) {
  return useConsoleLogin('feishu_account_switch', 'switch_poll', input, enabled);
}
export function useStartFeishuAccountSwitch() {
  return useConsoleLoginAction('feishu_account_switch', 'switch_start');
}
export function useCancelFeishuAccountSwitch() {
  return useConsoleLoginAction('feishu_account_switch', 'switch_cancel');
}
export function useFeishuConsoleLogin(input: DTO.GetWorkspaceRequest, enabled = true) {
  return useConsoleLogin('feishu_console_login', 'poll', input, enabled);
}
export function useStartFeishuConsoleLogin() {
  return useConsoleLoginAction('feishu_console_login', 'start');
}
// Agent onboarding polls from the state seeded by `start`, at the server's cadence.
const progressKey = (input: DTO.GetAgentOnboardingRequest) =>
  objectKey('agent_onboarding_progress', input);
export function useAgentOnboardingProgress(input: DTO.GetAgentOnboardingRequest, enabled = true) {
  const client = useQueryClient();
  const starting = useIsMutating({
    mutationKey: [...keys.onboarding, 'agent_onboarding_progress'],
    predicate: (mutation) => {
      const variables = mutation.state.variables as DTO.GetAgentOnboardingRequest | undefined;
      return (
        variables?.agent?.workspace_id === input.agent?.workspace_id &&
        variables?.agent?.agent_id === input.agent?.agent_id
      );
    },
  });
  const seeded = client.getQueryData(progressKey(input)) !== undefined;
  const poll = usePoll();
  return useQuery({
    queryKey: progressKey(input),
    queryFn: async ({ signal }): Promise<DTO.AgentOnboardingState | null> => {
      const previous = client.getQueryData<DTO.AgentOnboardingState>(progressKey(input));
      // Event invalidation must not advance a finished onboarding again.
      if (!previous || previous.status === 'ready' || previous.status === 'error')
        return previous ?? null;
      return call(
        'advance_agent_onboarding',
        {
          request_id: input.request_id,
          agent: input.agent,
          action: 'poll',
          field_values_json: previous.payload_json || '{}',
        },
        signal,
      );
    },
    enabled: enabled && seeded && starting === 0,
    refetchInterval: (query) => poll({ state: { data: query.state.data ?? undefined } }),
  });
}
export function useStartAgentOnboarding() {
  const client = useQueryClient();
  return useMutation({
    mutationKey: [...keys.onboarding, 'agent_onboarding_progress'],
    mutationFn: async (input: DTO.GetAgentOnboardingRequest) => {
      await client.cancelQueries({ queryKey: progressKey(input) });
      return call('advance_agent_onboarding', {
        ...input,
        action: 'start',
        field_values_json: '{}',
      });
    },
    onSuccess: async (state, input) => {
      await client.cancelQueries({ queryKey: progressKey(input) });
      client.setQueryData(progressKey(input), state);
    },
  });
}
export function useHarnessTrials(inputs: DTO.TestHarnessRequest[]) {
  return useQueries({
    queries: inputs.map((input) => ({
      queryKey: [...keys.onboarding, 'harness_trial', input],
      queryFn: ({ signal }: { signal: AbortSignal }) => call('test_harness', input, signal),
      // Settled answers are kept: event invalidation must not run the test again.
      enabled: (query: { state: { data: unknown } }) => query.state.data === undefined,
      retry: false,
    })),
  });
}
