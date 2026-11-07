import { expect, request as playwrightRequest } from '@playwright/test';
import { readFile, access } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { join } from 'node:path';
import { gunzipSync } from 'node:zlib';
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
  return `meerkat-machine pair wss://${access.address} ${code.code} --cert-sha256 ${access.cert_sha256}`;
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
test('Feishu registration expires, is denied, waits for permissions and enrolls once per creator', async ({
  baseURL,
}) => {
  const member = { workspace: 'feishu_rig', agent: 'alpha' };
  await rig({ op: 'create_workspace', workspace: member.workspace, provider: 'feishu' });
  await rig({ op: 'create_agent', ...member });
  const start = () => advance(baseURL!, member.workspace, member.agent, 'start');
  const poll = (payload: string) =>
    advance(baseURL!, member.workspace, member.agent, 'poll', payload);
  const first = await start();
  expect(first.verification_uri).toContain('/page/cli?user_code=');
  expect((await poll(first.payload_json)).status).toBe('action_required');
  await rig({ op: 'feishu_scan', workspace: member.workspace, state: 'expired' });
  expect((await poll(first.payload_json)).error_code).toBe('expired');
  const second = await start();
  expect(second.verification_uri).not.toBe(first.verification_uri);
  await rig({ op: 'feishu_scan', workspace: member.workspace, state: 'denied' });
  expect((await poll(second.payload_json)).error_code).toBe('access_denied');
  const third = await start();
  await rig({ op: 'feishu_scan', workspace: member.workspace, state: 'complete' });
  const permissions = await poll(third.payload_json);
  expect(permissions.verification_uri).toContain('/page/launcher?clientID=cli_registered_1');
  expect(permissions.step).toBe('permissions');
  const addons = new URL(permissions.verification_uri).searchParams.get('addons')!;
  const requested = JSON.parse(gunzipSync(Buffer.from(addons, 'base64url')).toString());
  expect(requested.scopes.user).toEqual([]);
  expect(requested.scopes.tenant).toEqual(
    expect.arrayContaining([
      'tenant:tenant:readonly',
      'contact:contact.base:readonly',
      'contact:user.base:readonly',
      'application:application:self_manage',
    ]),
  );
  expect(requested.events.items.tenant).toEqual(
    expect.arrayContaining(['im.message.receive_v1', 'task.task.comment.updated_v1']),
  );
  const waiting = await poll(permissions.payload_json);
  expect(waiting.status).toBe('verifying');
  expect((await poll(waiting.payload_json)).status).toBe('verifying');
  const before = await api(baseURL!, 'list_human_members', {
    workspace: { workspace_id: member.workspace },
  });
  expect(before).toHaveLength(0);
  await rig({ op: 'feishu_scan', workspace: member.workspace, state: 'approved' });
  expect((await poll(waiting.payload_json)).status).toBe('ready');
  await rig({ op: 'dm', ...member, text: 'hello' });
  await rig({ op: 'settle' });
  const reply = await api(baseURL!, 'get_agent_first_reply', {
    agent: { workspace_id: member.workspace, agent_id: member.agent },
  });
  expect(reply.state).toBe('replied');
  await rig({ op: 'create_agent', workspace: member.workspace, agent: 'beta' });
  await rig({ op: 'feishu_scan', workspace: member.workspace, state: 'waiting' });
  const next = await advance(baseURL!, member.workspace, 'beta', 'start');
  expect(next.verification_uri).not.toBe(third.verification_uri);
  expect(
    (await advance(baseURL!, member.workspace, 'beta', 'poll', next.payload_json)).status,
  ).toBe('action_required');
  await rig({ op: 'feishu_scan', workspace: member.workspace, state: 'approved' });
  const created = await advance(baseURL!, member.workspace, 'beta', 'poll', next.payload_json);
  expect(
    (await advance(baseURL!, member.workspace, 'beta', 'poll', created.payload_json)).status,
  ).toBe('ready');
  const humans = await api(baseURL!, 'list_human_members', {
    workspace: { workspace_id: member.workspace },
  });
  expect(humans).toHaveLength(1);
  expect(humans[0].team_role).toBe('lead');
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

test('Telegram member creation from the board starts onboarding', async ({ page, baseURL }) => {
  const workspace = 'tg_members';
  await rig({ op: 'create_workspace', workspace, provider: 'telegram' });
  await page.addInitScript((ws) => {
    localStorage.setItem('meerkat.workspace', JSON.stringify(ws));
  }, workspace);
  await page.goto('/#/member/new');
  await page.getByTestId('member-name').fill('Ada');
  const created = page.waitForResponse((response) => response.url().endsWith('/create_agent'));
  const started = page.waitForResponse(
    (response) =>
      response.url().endsWith('/advance_agent_onboarding') &&
      response.request().postDataJSON().action === 'start',
  );
  await page.getByTestId('member-create').click();
  const saved = await (await created).json();
  const agent_id = saved.agent.agent_id;
  expect(JSON.parse(saved.provider_agent_config_json)).toEqual({
    mode: 'new',
    bot_username: `${workspace}_${agent_id}_bot`,
  });
  const onboarding = await (await started).json();
  expect(onboarding.status).toBe('action_required');
  expect(onboarding.verification_uri).toContain(
    `https://t.me/newbot/manager_bot/${workspace}_${agent_id}_bot`,
  );
  await expect(page.getByTestId('member-create-qr')).toHaveAttribute('data-state', 'wait');
  await expect(page.getByText('Scan with Telegram', { exact: true })).toBeVisible();
  const persisted = await api(baseURL!, 'get_agent', {
    agent: { workspace_id: workspace, agent_id },
  });
  expect(JSON.parse(persisted.provider_agent_config_json).bot_username).toBe(
    `${workspace}_${agent_id}_bot`,
  );
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
