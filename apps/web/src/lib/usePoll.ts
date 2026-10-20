import type { AgentOnboardingState, ProviderOnboardingState } from './api/generated';
type PollingState =
  | AgentOnboardingState
  | ProviderOnboardingState
  | { state: 'required' | 'ready' | 'waiting' | 'scanned'; poll_interval_seconds?: number };
// Onboarding and account switching use the cadence returned by the server.
export function usePoll() {
  return (query: { state: { data: PollingState | undefined } }) => {
    const state = query.state.data;
    if (!state) return false;
    const pending =
      'status' in state
        ? state.status !== 'ready' && state.status !== 'error'
        : state.state === 'waiting' || state.state === 'scanned';
    const interval = state.poll_interval_seconds;
    return pending && interval !== undefined && interval > 0 ? interval * 1000 : false;
  };
}
