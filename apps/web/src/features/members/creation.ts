import { ApiError } from '../../lib/api/client';
import type * as DTO from '../../lib/api/generated';

// Where a new member is while its chat account is being set up.
export type Creation =
  | { step: 'idle' }
  | { step: 'creating' }
  | { step: 'confirm'; qr: string }
  | { step: 'permissions'; url: string }
  | { step: 'failed'; enterprise?: boolean }
  | { step: 'ready'; id: string };

// Channels that give each member its own app set it up while the member is created.
export const onboards = (provider: string | undefined) => provider === 'feishu';

export function creationOf(
  id: string,
  state: DTO.AgentOnboardingState | undefined,
  error: unknown,
): Creation {
  if (error || state?.status === 'error')
    return {
      step: 'failed',
      enterprise: error instanceof ApiError && error.detail === 'team_enterprise',
    };
  if (state?.status === 'ready') return { step: 'ready', id };
  if (state?.step === 'permissions' && state.verification_uri)
    return { step: 'permissions', url: state.verification_uri };
  const qr = state?.verification_uri;
  return qr ? { step: 'confirm', qr } : { step: 'creating' };
}
