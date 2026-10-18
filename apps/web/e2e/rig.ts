import { test as base } from '@playwright/test';
import { connect } from 'node:net';
import { createInterface } from 'node:readline';

type Member = { workspace: string; agent: string };
// This is deliberately limited to the mock IM / scripted Harness scenarios.
export type Command =
  | {
      op: 'create_workspace';
      workspace: string;
      provider?: 'system_im' | 'feishu' | 'telegram';
      owner_avatar?: boolean;
    }
  | { op: 'set_duty'; workspace: string; on: boolean }
  | ({ op: 'create_agent' | 'hold_next_run' | 'release_run' | 'wait_running' } & Member)
  | ({ op: 'mention'; chat: string; person: string; text: string } & Member)
  | { op: 'pair_machine'; command: string }
  | { op: 'feishu_scan'; workspace: string; state: 'waiting' | 'scanned' | 'expired' | 'complete' }
  | ({ op: 'dm'; text: string } & Member)
  | { op: 'feishu_task'; workspace: string; text: string }
  | { op: 'settle' };

type Reply = {
  machine_id?: string;
  outbound?: {
    workspace: string;
    account: string;
    chat: string;
    channel_type: string;
    content: unknown;
  }[];
};

export async function rig(
  command: Command,
  port = Number(process.env.LANKIDE_WEB_CONTROL_PORT),
): Promise<Reply> {
  if (!port) throw new Error('Playwright globalSetup did not start the web rig');
  const socket = connect({ host: '127.0.0.1', port });
  const lines = createInterface({ input: socket });
  try {
    return await new Promise<Reply>((resolve, reject) => {
      socket.setTimeout(35_000, () => reject(new Error(`Rig timed out: ${command.op}`)));
      socket.once('error', reject);
      lines.once('close', () => reject(new Error(`Rig closed: ${command.op}`)));
      lines.once('line', (line) => {
        try {
          const reply = JSON.parse(line) as { ok: boolean; error?: string; data: Reply };
          if (reply.ok) resolve(reply.data);
          else reject(new Error(reply.error));
        } catch (error) {
          reject(error);
        }
      });
      socket.once('connect', () => socket.write(`${JSON.stringify(command)}\n`));
    });
  } finally {
    lines.close();
    socket.destroy();
  }
}

export const test = base.extend({
  baseURL: async ({}, use) => {
    const url = process.env.LANKIDE_WEB_BASE_URL;
    if (!url) throw new Error('Web rig URL is missing');
    await use(url);
  },
});
