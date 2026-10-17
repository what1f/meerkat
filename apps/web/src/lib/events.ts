import { useEffect } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import type { EventFrame } from './api/generated';
import { eventKeys } from './queries/keys';

export function useManagementEvents(workspace?: string) {
  const client = useQueryClient();
  useEffect(() => {
    let stopped = false;
    let socket: WebSocket;
    let timer: ReturnType<typeof setTimeout>;
    let failures = 0;
    function connect() {
      const url = new URL('/api/v1/events', location.href);
      url.protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
      if (workspace) url.searchParams.set('workspace', workspace);
      // A daemon restart resets sequences; subscriptions have no replay.
      url.searchParams.set('after', '0');
      socket = new WebSocket(url);
      socket.onopen = () => {
        failures = 0;
        void client.invalidateQueries();
      };
      socket.onmessage = ({ data }) => {
        let frame: EventFrame;
        try {
          frame = JSON.parse(data);
        } catch {
          socket.close();
          return;
        }
        if (frame.type === 'error') {
          socket.close();
          return;
        }
        for (const queryKey of eventKeys[frame.data.kind]) {
          void client.invalidateQueries({ queryKey });
        }
      };
      socket.onerror = () => socket.close();
      socket.onclose = () => {
        if (!stopped) timer = setTimeout(connect, Math.min(1000 * 2 ** failures++, 30000));
      };
    }
    connect();
    return () => {
      stopped = true;
      clearTimeout(timer);
      socket.close();
    };
  }, [client, workspace]);
}
