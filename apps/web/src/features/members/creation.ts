import type * as DTO from '../../lib/api/generated';

// Where a new member is while its chat account is being set up.
export type Creation =
  | { step: 'idle' }
  | { step: 'creating' }
  | { step: 'confirm'; qr: string; scanned: boolean }
  | { step: 'failed' }
  | { step: 'ready'; id: string };

// Channels that give each member its own app set it up while the member is created.
export const onboards = (provider: string | undefined) => provider === 'feishu';

export function creationOf(
  id: string,
  state: DTO.AgentOnboardingState | undefined,
  failed: boolean,
): Creation {
  if (failed || state?.status === 'error') return { step: 'failed' };
  if (state?.status === 'ready') return { step: 'ready', id };
  const payload = state
    ? (JSON.parse(state.payload_json || '{}') as { qr_code?: string; verification_uri?: string })
    : {};
  const qr = payload.qr_code || payload.verification_uri || state?.verification_uri;
  return qr
    ? { step: 'confirm', qr, scanned: state?.status === 'verifying' }
    : { step: 'creating' };
}
