import type { AgentOnboardingState, ProviderOnboardingState } from './api/generated';
// Only onboarding (including Feishu QR login) has a server-defined polling cadence.
export function usePoll() {
  return (query: {
    state: { data: AgentOnboardingState | ProviderOnboardingState | undefined };
  }) => {
    const state = query.state.data;
    return state &&
      state.status !== 'ready' &&
      state.status !== 'error' &&
      state.poll_interval_seconds > 0
      ? state.poll_interval_seconds * 1000
      : false;
  };
}
