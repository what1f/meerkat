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
// One console sign-in interface serves both flows: `poll` shows a code, follows
// the scan and replaces an expired code; `start` always shows a new one.
type ConsoleLoginAction = 'start' | 'poll' | 'switch_start' | 'switch_poll' | 'switch_cancel';
type ConsoleLoginFlow = 'feishu_account_switch' | 'feishu_console_login';
export type FeishuAccountSwitch = {
  state: 'required' | 'ready' | 'waiting' | 'scanned';
  qr_code?: string;
  poll_interval_seconds?: number;
};
// The request id does not change what a read returns.
const objectKey = (name: string, input: object) => [
  ...keys.onboarding,
  name,
  { ...input, request_id: undefined },
];
const loginKey = objectKey;
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
  return useQuery({
    queryKey: loginKey(flow, input),
    queryFn: ({ signal }) => consoleLogin(input, action, signal),
    enabled: enabled && pending === 0,
    refetchInterval: usePoll(),
  });
}
function useConsoleLoginAction(flow: ConsoleLoginFlow, action: ConsoleLoginAction) {
  const client = useQueryClient();
  return useMutation({
    mutationKey: [...keys.onboarding, flow],
    mutationFn: async (input: DTO.GetWorkspaceRequest) => {
      await client.cancelQueries({ queryKey: loginKey(flow, input) });
      return consoleLogin(input, action);
    },
    onSuccess: async (state, input) => {
      await client.cancelQueries({ queryKey: loginKey(flow, input) });
      client.setQueryData(loginKey(flow, input), state);
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
// Workspace sign-in before the first member.
export function useFeishuConsoleLogin(input: DTO.GetWorkspaceRequest, enabled = true) {
  return useConsoleLogin('feishu_console_login', 'poll', input, enabled);
}
export function useStartFeishuConsoleLogin() {
  return useConsoleLoginAction('feishu_console_login', 'start');
}
// Agent onboarding moves only on advance: `start` seeds the state, then the
// query polls with the last payload at the server's cadence until it settles.
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
// Onboarding tries each AI the owner picks once and keeps the answer.
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
