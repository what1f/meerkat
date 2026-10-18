import { expect, request as playwrightRequest } from '@playwright/test';
import { readFile, access } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { join } from 'node:path';
import { test, rig } from './rig';
import { startRig } from './rig-process';

async function api(baseURL: string, op: string, body: object = {}) {
  const client = await playwrightRequest.newContext({ baseURL });
  try {
    const response = await client.post(`/api/v1/${op}`, { data: { request_id: op, ...body } });
    expect(response.ok(), await response.text()).toBe(true);
    return await response.json();
  } finally {
    await client.dispose();
  }
}

async function pairingCommand(baseURL: string) {
  const detected = await api(baseURL, 'enable_remote_access');
  const port = new URL(`wss://${detected.address}`).port;
  const access = await api(baseURL, 'set_remote_access', { address: `127.0.0.1:${port}` });
  const code = await api(baseURL, 'create_pairing_code');
  return `lankide-machine pair wss://${access.address} ${code.code} --cert-sha256 ${access.cert_sha256}`;
}

async function moveMember(baseURL: string, workspace: string, agent: string, machine_id: string) {
  await api(baseURL, 'update_agent', {
    agent: { workspace_id: workspace, agent_id: agent },
    machine_id,
  });
}

test('pair the displayed command and run the moved member on that Machine', async ({ baseURL }) => {
  const member = { workspace: 'placement_rig', agent: 'bruce' };
  await rig({ op: 'create_workspace', workspace: member.workspace });
  await rig({ op: 'create_agent', ...member });
  const paired = await rig({ op: 'pair_machine', command: await pairingCommand(baseURL!) });
  expect(paired.machine_id).toBeTruthy();
  await moveMember(baseURL!, member.workspace, member.agent, paired.machine_id!);
  await rig({ op: 'hold_next_run', ...member });
  await rig({
    op: 'mention',
    ...member,
    chat: 'group',
    person: 'alice',
    text: 'work on the new Machine',
  });
  const running = await rig({ op: 'wait_running', ...member });
  expect(running.machine_id).toBe(paired.machine_id);
  await rig({ op: 'release_run', ...member });
  await rig({ op: 'settle' });
});

test('owner DM produces a private reply distinct from group messages', async () => {
  const member = { workspace: 'dm_rig', agent: 'bruce' };
  await rig({ op: 'create_workspace', workspace: member.workspace });
  await rig({ op: 'create_agent', ...member });
  await rig({ op: 'dm', ...member, text: 'hello from the owner' });
  const result = await rig({ op: 'settle' });
  expect(result.outbound).toContainEqual(
    expect.objectContaining({
      workspace: member.workspace,
      account: 'bot-bruce',
      chat: 'owner',
      channel_type: 'private_chat',
      content: 'hi',
    }),
  );
});

test('timeout teardown kills independent Harness groups before deleting every home', async () => {
  test.setTimeout(60_000);
  const running = await startRig();
  const port = running.controlPort;
  const member = { workspace: 'cleanup_rig', agent: 'bruce' };
  try {
    await rig({ op: 'create_workspace', workspace: member.workspace }, port);
    await rig({ op: 'create_agent', ...member }, port);
    const remote = await rig(
      { op: 'pair_machine', command: await pairingCommand(running.baseURL) },
      port,
    );
    await moveMember(running.baseURL, member.workspace, member.agent, remote.machine_id!);
    await rig({ op: 'hold_next_run', ...member }, port);
    await rig({ op: 'dm', ...member, text: 'hold this run during shutdown' }, port);
    await rig({ op: 'wait_running', ...member }, port);
    const journal = await readFile(
      join(running.runtimeHome, 'web-machines/0/harness-pids.tsv'),
      'utf8',
    );
    const peers = journal.split('\n').flatMap((line) => {
      const match = /^spawned\t(\d+)$/.exec(line);
      return match ? [Number(match[1])] : [];
    });
    expect(peers.length).toBeGreaterThan(0);
    process.kill(running.pid, 'SIGSTOP');
    await expect(running.cleanup()).rejects.toThrow('Web rig shutdown timed out');
    for (const pid of peers) {
      let state = '';
      try {
        state = execFileSync('ps', ['-o', 'stat=', '-p', String(pid)], { encoding: 'utf8' }).trim();
      } catch {
        /* Missing PID is the expected outcome. */
      }
      expect(state === '' || state.startsWith('Z')).toBe(true);
    }
    await expect(access(running.runtimeHome)).rejects.toThrow();
    await expect(access(running.machineHome)).rejects.toThrow();
  } finally {
    await running.cleanup().catch(() => {});
  }
});

async function advance(
  baseURL: string,
  workspace: string,
  agent: string,
  action: string,
  payload = '{}',
) {
  return api(baseURL, 'advance_agent_onboarding', {
    agent: { workspace_id: workspace, agent_id: agent },
    action,
    field_values_json: payload,
  });
}
async function consoleLogin(baseURL: string, workspace: string, action: string) {
  const reply = await api(baseURL, 'invoke_provider_interface', {
    workspace: { workspace_id: workspace },
    interface_id: 'feishu.workspace.console_login',
    input_json: JSON.stringify({ action }),
  });
  return JSON.parse(reply.output_json);
}

test('Feishu scan mock drives waiting, scanned, expiry and completion through the real Provider', async ({
  baseURL,
}) => {
  const member = { workspace: 'feishu_rig', agent: 'alpha' };
  await rig({ op: 'create_workspace', workspace: member.workspace, provider: 'feishu' });
  await rig({ op: 'feishu_scan', workspace: member.workspace, state: 'waiting' });
  const waiting = await consoleLogin(baseURL!, member.workspace, 'start');
  expect(waiting.state).toBe('waiting');
  expect((await consoleLogin(baseURL!, member.workspace, 'poll')).state).toBe('waiting');
  await rig({ op: 'feishu_scan', workspace: member.workspace, state: 'scanned' });
  expect((await consoleLogin(baseURL!, member.workspace, 'poll')).state).toBe('scanned');
  await rig({ op: 'feishu_scan', workspace: member.workspace, state: 'expired' });
  const expired = await consoleLogin(baseURL!, member.workspace, 'poll');
  expect(expired.state).toBe('waiting');
  expect(expired.qr_code).toBe('');
  const replacement = await consoleLogin(baseURL!, member.workspace, 'poll');
  expect(replacement.state).toBe('waiting');
  expect(replacement.qr_code).toBeTruthy();
  expect(replacement.qr_code).not.toBe(waiting.qr_code);
  await rig({ op: 'feishu_scan', workspace: member.workspace, state: 'complete' });
  expect((await consoleLogin(baseURL!, member.workspace, 'poll')).state).toBe('ready');
  await rig({ op: 'create_agent', ...member });
  await advance(baseURL!, member.workspace, member.agent, 'start');
  const ready = await advance(baseURL!, member.workspace, member.agent, 'start');
  expect(ready.status).toBe('ready');
  await rig({ op: 'dm', ...member, text: 'hello from the owner' });
  const result = await rig({ op: 'settle' });
  expect(result.outbound).toContainEqual(
    expect.objectContaining({
      workspace: member.workspace,
      channel_type: 'private_chat',
    }),
  );
  const reply = await api(baseURL!, 'get_agent_first_reply', {
    agent: { workspace_id: member.workspace, agent_id: member.agent },
  });
  expect(reply.state).toBe('replied');
  // The first App consumed this scan. Another App must show a new QR code.
  await rig({ op: 'create_agent', workspace: member.workspace, agent: 'beta' });
  const next = await advance(baseURL!, member.workspace, 'beta', 'start');
  expect(next.status).toBe('action_required');
  expect(JSON.parse(next.payload_json).qr_code).toBeTruthy();
  await rig({ op: 'feishu_scan', workspace: member.workspace, state: 'complete' });
  const created = await advance(baseURL!, member.workspace, 'beta', 'poll', next.payload_json);
  expect(
    (await advance(baseURL!, member.workspace, 'beta', 'poll', created.payload_json)).status,
  ).toBe('ready');
  const humans = await api(baseURL!, 'list_human_members', {
    workspace: { workspace_id: member.workspace },
  });
  expect(humans).toHaveLength(1);
});

test('Telegram Token completes real onboarding and owner DM records the first private reply', async ({
  baseURL,
}) => {
  const member = { workspace: 'telegram_rig', agent: 'alpha' };
  await api(baseURL!, 'create_workspace', {
    workspace_id: member.workspace,
    name: 'Telegram',
    rules: '',
    provider_id: 'telegram',
    provider_config_json: '{"poll_timeout_seconds":1}',
    provider_secrets_json: '{"manager_bot_token":"100:manager"}',
  });
  await rig({ op: 'create_agent', ...member });
  await advance(baseURL!, member.workspace, member.agent, 'start');
  expect((await advance(baseURL!, member.workspace, member.agent, 'poll')).status).toBe('ready');
  const agent = { workspace_id: member.workspace, agent_id: member.agent };
  expect((await api(baseURL!, 'get_agent_first_reply', { agent })).state).toBe('waiting');
  await rig({ op: 'dm', ...member, text: 'hello from the Telegram owner' });
  const result = await rig({ op: 'settle' });
  expect(result.outbound).toContainEqual(
    expect.objectContaining({
      workspace: member.workspace,
      channel_type: 'private_chat',
      content: 'hi',
    }),
  );
  expect((await api(baseURL!, 'get_agent_first_reply', { agent })).state).toBe('replied');
});

async function createPageMember(
  baseURL: string,
  workspace: string,
  agent: string,
  provider = 'system_im',
) {
  const catalog = await api(baseURL, 'get_machine_catalog');
  const machine = catalog.machines.find((m: { harnesses: { availability: string }[] }) =>
    m.harnesses.some((h) => h.availability === 'available'),
  );
  const harness = machine.harnesses.find(
    (h: { availability: string }) => h.availability === 'available',
  );
  // Use a catalog model different from the scripted fixture's historical model.
  const model = harness.model_catalog.models.find(
    (m: { model_id: string }) => m.model_id === 'short',
  );
  return api(baseURL, 'create_agent', {
    agent: { workspace_id: workspace, agent_id: agent },
    name: agent,
    role: '',
    bio: '',
    figure: '',
    launch_arguments: [],
    machine_id: machine.machine_id,
    harness_id: harness.harness_id,
    model: model.model_id,
    thinking_level: model.supported_thinking_levels[0],
    auto_reaction: '',
    soul: '',
    cli_path: '',
    environment_variables: [],
    attention: null,
    provider_agent_config_json: JSON.stringify(
      provider === 'system_im'
        ? { account: `bot-${agent}` }
        : { mode: 'new', bot_username: 'alpha_bot' },
    ),
    provider_agent_secrets_json: '{}',
  });
}

test('page API member runs with empty environment and preserves its model through hold and release', async ({
  baseURL,
}) => {
  const member = { workspace: 'page_member', agent: 'bruce' };
  await rig({ op: 'create_workspace', workspace: member.workspace });
  const created = await createPageMember(baseURL!, member.workspace, member.agent);
  expect(created.environment_variables).toEqual([]);
  expect(created.cli_path).toBe('');
  await expect(
    rig({ op: 'hold_next_run', workspace: member.workspace, agent: 'missing' }),
  ).rejects.toThrow();
  await expect(
    rig({ op: 'release_run', workspace: member.workspace, agent: 'missing' }),
  ).rejects.toThrow();
  // Repeat on the same native session to cover re-arming the file gate.
  for (let n = 0; n < 2; n++) {
    await rig({ op: 'hold_next_run', ...member });
    await rig({
      op: 'mention',
      ...member,
      chat: 'page_group',
      person: 'alice',
      text: `hello ${n}`,
    });
    expect((await rig({ op: 'wait_running', ...member })).machine_id).toBe(created.machine_id);
    await rig({ op: 'release_run', ...member });
    const result = await rig({ op: 'settle' });
    expect(result.outbound).toContainEqual(
      expect.objectContaining({
        workspace: member.workspace,
        channel_type: 'group_chat',
        content: 'hi',
      }),
    );
    const saved = await api(baseURL!, 'get_agent', {
      agent: { workspace_id: member.workspace, agent_id: member.agent },
    });
    expect(saved.model).toBe(created.model);
    expect(saved.environment_variables).toEqual([]);
    expect(saved.cli_path).toBe('');
  }
});

test('page API Telegram member completes onboarding and receives its first owner DM reply', async ({
  baseURL,
}) => {
  const member = { workspace: 'page_dm', agent: 'alpha' };
  await rig({ op: 'create_workspace', workspace: member.workspace, provider: 'telegram' });
  await createPageMember(baseURL!, member.workspace, member.agent, 'telegram');
  await advance(baseURL!, member.workspace, member.agent, 'start');
  expect((await advance(baseURL!, member.workspace, member.agent, 'poll')).status).toBe('ready');
  const agent = { workspace_id: member.workspace, agent_id: member.agent };
  expect((await api(baseURL!, 'get_agent_first_reply', { agent })).state).toBe('waiting');
  await rig({ op: 'dm', ...member, text: 'first owner message for a page-created member' });
  const result = await rig({ op: 'settle' });
  expect(result.outbound).toContainEqual(
    expect.objectContaining({
      workspace: member.workspace,
      channel_type: 'private_chat',
      content: 'hi',
    }),
  );
  expect((await api(baseURL!, 'get_agent_first_reply', { agent })).state).toBe('replied');
});

test('off-duty IM accepts mention and DM without replies, then new messages get replies on duty', async ({
  baseURL,
}) => {
  const member = { workspace: 'shift_rig', agent: 'bruce' };
  const workspace = { workspace_id: member.workspace };
  await rig({ op: 'create_workspace', workspace: member.workspace });
  await rig({ op: 'create_agent', ...member });
  const mention = {
    op: 'mention' as const,
    ...member,
    chat: 'group',
    person: 'alice',
    text: 'hello while off duty',
  };
  await api(baseURL!, 'set_workspace_state', { workspace, desired_state: 'paused' });
  await rig(mention);
  await rig({ op: 'dm', ...member, text: 'owner message while off duty' });
  const replies = (result: Awaited<ReturnType<typeof rig>>) =>
    result.outbound!.filter((message) => message.workspace === member.workspace);
  expect(replies(await rig({ op: 'settle' }))).toEqual([]);

  await api(baseURL!, 'set_workspace_state', { workspace, desired_state: 'running' });
  expect(replies(await rig({ op: 'settle' }))).toEqual([]);
  await rig({ ...mention, text: 'hello back on duty' });
  expect(replies(await rig({ op: 'settle' }))).toEqual([
    expect.objectContaining({ chat: 'group', channel_type: 'group_chat', content: 'hi' }),
  ]);
  await rig({ op: 'dm', ...member, text: 'owner message back on duty' });
  expect(replies(await rig({ op: 'settle' }))).toEqual([
    expect.objectContaining({ chat: 'group', channel_type: 'group_chat', content: 'hi' }),
    expect.objectContaining({ chat: 'owner', channel_type: 'private_chat', content: 'hi' }),
  ]);
});
