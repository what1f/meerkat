import { useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { call } from '../api/client';
import type * as DTO from '../api/generated';
import { keys } from './keys';

type Start = DTO.BuiltinAgentSignInRequest;

// One sign-in at a time per picker: start it, open the browser step, then watch it finish.
export function useBuiltinSignIn(onSignedIn?: () => void) {
  const client = useQueryClient();
  const [id, setId] = useState<string | null>(null);
  const start = useMutation({
    mutationFn: (input: Start) => call('builtin_agent_sign_in', input),
    onSuccess: (result) => {
      if (result.authorize_url) window.open(result.authorize_url, '_blank');
      setId(result.sign_in_id);
    },
  });
  const status = useQuery({
    queryKey: ['builtin_agent_sign_in', id],
    queryFn: async ({ signal }) => {
      const result = await call('builtin_agent_sign_in_status', { sign_in_id: id! }, signal);
      if (result.state === 'succeeded') {
        setId(null);
        void client.invalidateQueries({ queryKey: keys.machines });
        void client.invalidateQueries({ queryKey: keys.catalogs });
        // The account decides which models there are.
        void client.invalidateQueries({ queryKey: ['models'] });
        onSignedIn?.();
      }
      return result;
    },
    enabled: id !== null,
    refetchInterval: (query) =>
      query.state.data?.state === 'pending' || !query.state.data ? 1000 : false,
    gcTime: 0,
  });
  const state = status.data?.state;
  const cancel = () => {
    if (id) void call('builtin_agent_sign_in_cancel', { sign_in_id: id });
    setId(null);
    start.reset();
  };
  return {
    start: start.mutate,
    cancel,
    waiting: start.isPending || (id !== null && state !== 'failed' && state !== 'expired'),
    failed: start.isError || state === 'failed' || state === 'expired',
    reason: start.isError ? null : (status.data?.reason ?? null),
  };
}

export function useBuiltinSignOut() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: (machine_id: string) =>
      call('builtin_agent_sign_out', { workspace: null, machine_id }),
    onSuccess: async () => {
      await client.invalidateQueries({ queryKey: keys.machines });
    },
  });
}
