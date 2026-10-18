import { expect, request, type Page } from '@playwright/test';
import { test, rig } from './rig';

async function api(baseURL: string, op: string, body: object = {}) {
  const client = await request.newContext({ baseURL });
  try {
    const response = await client.post(`/api/v1/${op}`, { data: { request_id: op, ...body } });
    expect(response.ok(), await response.text()).toBe(true);
    return await response.json();
  } finally {
    await client.dispose();
  }
}
const workspaceOf = (baseURL: string, workspace_id: string) =>
  api(baseURL, 'get_workspace', { workspace: { workspace_id } });

async function signInFeishu(baseURL: string, workspace: string) {
  await rig({ op: 'create_workspace', workspace, provider: 'feishu' });
  const login = (action: string) =>
    api(baseURL, 'invoke_provider_interface', {
      workspace: { workspace_id: workspace },
      interface_id: 'feishu.workspace.console_login',
      input_json: JSON.stringify({ action }),
    });
  await rig({ op: 'feishu_scan', workspace, state: 'waiting' });
  await login('start');
  await rig({ op: 'feishu_scan', workspace, state: 'complete' });
  expect(JSON.parse((await login('poll')).output_json).state).toBe('ready');
}
// The first member's onboarding binds the owner who keeps the team task list.
async function bindOwner(baseURL: string, workspace: string) {
  await rig({ op: 'create_agent', workspace, agent: 'alpha' });
  for (const status of ['verifying', 'ready'])
    expect(
      (
        await api(baseURL, 'advance_agent_onboarding', {
          agent: { workspace_id: workspace, agent_id: 'alpha' },
          action: 'start',
          field_values_json: '{}',
        })
      ).status,
    ).toBe(status);
}
const tasksEnabled = async (baseURL: string, workspace: string) =>
  JSON.parse((await workspaceOf(baseURL, workspace)).provider_config_json).task_system_enabled;

async function openTeam(page: Page, workspace: string) {
  await page.addInitScript((id) => {
    localStorage.setItem('lankide.workspace', JSON.stringify(id));
  }, workspace);
  await page.goto('/#/team');
  await expect(page.getByTestId('team-name')).toContainText(workspace);
}

async function replies(workspace: string) {
  const { outbound } = await rig({ op: 'settle' });
  return outbound!.filter((m) => m.workspace === workspace && m.chat === 'group');
}

test('rules typed on the tag are saved when the field loses focus', async ({ page, baseURL }) => {
  await rig({ op: 'create_workspace', workspace: 'rules_team' });
  await openTeam(page, 'rules_team');
  const rules = page.getByTestId('team-rules');
  await rules.fill('Reply in Chinese.\nKeep answers short.');
  expect((await workspaceOf(baseURL!, 'rules_team')).rules).toBe('');
  await rules.blur();
  await expect
    .poll(async () => (await workspaceOf(baseURL!, 'rules_team')).rules)
    .toBe('Reply in Chinese.\nKeep answers short.');
  await page.reload();
  await expect(page.getByTestId('team-rules')).toHaveValue(
    'Reply in Chinese.\nKeep answers short.',
  );
});

test('the team name is renamed in place and the sidebar follows', async ({ page, baseURL }) => {
  await rig({ op: 'create_workspace', workspace: 'rename_team' });
  await openTeam(page, 'rename_team');
  await page.getByTestId('team-name').getByRole('button').click();
  await page.getByTestId('team-name').getByRole('textbox').fill('Night Desk');
  await page.keyboard.press('Enter');
  await expect(
    page.getByRole('complementary').getByRole('button', { name: 'Night Desk' }),
  ).toBeVisible();
  expect((await workspaceOf(baseURL!, 'rename_team')).name).toBe('Night Desk');
});

test('off shift members stop replying and answer again once back on', async ({ page, baseURL }) => {
  const member = { workspace: 'shift_team', agent: 'bruce' };
  await rig({ op: 'create_workspace', workspace: member.workspace });
  await rig({ op: 'create_agent', ...member });
  await openTeam(page, member.workspace);
  const mention = (text: string) =>
    rig({ op: 'mention', ...member, chat: 'group', person: 'alice', text });
  await mention('before the shift ends');
  expect(await replies(member.workspace)).toHaveLength(1);

  const shift = page.getByTestId('team-shift').getByRole('switch');
  await expect(shift).toHaveAttribute('aria-checked', 'true');
  await shift.click();
  await expect(shift).toHaveAttribute('aria-checked', 'false');
  await expect
    .poll(async () => (await workspaceOf(baseURL!, member.workspace)).observed_state)
    .toBe('paused');
  // A closed team's IM connection is stopped, so the message may not even be accepted.
  await mention('anyone there after hours?').catch(() => {});
  expect(await replies(member.workspace)).toHaveLength(1);

  await shift.click();
  await expect(shift).toHaveAttribute('aria-checked', 'true');
  await expect
    .poll(async () => (await workspaceOf(baseURL!, member.workspace)).observed_state)
    .toBe('running');
  await mention('good morning');
  await expect.poll(async () => (await replies(member.workspace)).length).toBeGreaterThan(1);
  await page.reload();
  await expect(page.getByTestId('team-shift').getByRole('switch')).toHaveAttribute(
    'aria-checked',
    'true',
  );
});

test('the Feishu task list turns on, takes new tasks, ticks them off and turns off', async ({
  page,
  baseURL,
}) => {
  const workspace = 'tasks_team';
  await signInFeishu(baseURL!, workspace);
  await bindOwner(baseURL!, workspace);
  await openTeam(page, workspace);

  const list = page.getByTestId('tasklist');
  await list.click();
  const input = list.getByRole('textbox');
  await input.fill('Draft the launch notes');
  await input.press('Enter');
  const row = list.getByText('Draft the launch notes');
  await expect(row).toBeVisible();
  await list.getByRole('checkbox').first().click();
  await expect(list.getByRole('checkbox').first()).toBeChecked();
  await expect
    .poll(async () => {
      const reply = await api(baseURL!, 'invoke_provider_interface', {
        workspace: { workspace_id: workspace },
        interface_id: 'feishu.workspace.tasks',
        input_json: JSON.stringify({ action: 'list' }),
      });
      return JSON.parse(reply.output_json).items[0]?.completed;
    })
    .toBe(true);
  await page.reload();
  await expect(page.getByTestId('tasklist').getByRole('checkbox').first()).toBeChecked();

  await page.getByTestId('tasklist').getByRole('button', { name: 'More' }).click();
  await page.getByRole('menuitem', { name: 'Turn off' }).click();
  await expect(page.getByTestId('tasklist')).not.toContainText('Draft the launch notes');
  expect(await tasksEnabled(baseURL!, workspace)).toBe(false);
});

test('a task list turned on before the owner is bound can be retried or turned off', async ({
  page,
  baseURL,
}) => {
  // Each failed read is retried a few times before the list gives up.
  test.setTimeout(90_000);
  const workspace = 'unbound_team';
  await signInFeishu(baseURL!, workspace);
  await openTeam(page, workspace);
  const list = page.getByTestId('tasklist');
  const retry = list.getByRole('button', { name: 'Try again' });

  await list.click();
  await expect.poll(() => tasksEnabled(baseURL!, workspace)).toBe(true);
  await expect(list).toContainText('The task list can’t be opened right now');
  await expect(retry).toBeVisible();
  await list.getByRole('button', { name: 'More' }).click();
  await page.getByRole('menuitem', { name: 'Turn off' }).click();
  await expect(retry).toBeHidden();
  await expect(list).toHaveText('Task list');
  expect(await tasksEnabled(baseURL!, workspace)).toBe(false);

  await list.click();
  await expect(retry).toBeVisible();
  // After a reload nothing was ever read; retrying keeps the clipboard and its way to turn off.
  await page.reload();
  await expect(retry).toBeVisible();
  await retry.click();
  await expect(retry).toHaveAttribute('aria-busy', 'true');
  await expect(list.getByRole('button', { name: 'More' })).toBeVisible();
  await expect(retry).toHaveAttribute('aria-busy', 'false');
  await expect(list).toContainText('The task list can’t be opened right now');
  // Once the owner is bound the list comes back on its own.
  await bindOwner(baseURL!, workspace);
  const input = list.getByRole('textbox');
  await input.fill('Book the venue');
  await input.press('Enter');
  await expect(list.getByText('Book the venue')).toBeVisible();
});

test('pieces moved on the wall stay where they were hung, per team', async ({ page }) => {
  await rig({ op: 'create_workspace', workspace: 'wall_team' });
  await rig({ op: 'create_workspace', workspace: 'wall_other' });
  await openTeam(page, 'wall_team');
  const sign = page.getByTestId('team-shift');
  const before = (await sign.boundingBox())!;
  await page.mouse.move(before.x + before.width / 2, before.y + 4);
  await page.mouse.down();
  await page.mouse.move(before.x + before.width / 2 + 60, before.y + 200, { steps: 12 });
  await page.mouse.up();
  await expect.poll(async () => (await sign.boundingBox())!.y).toBeGreaterThan(before.y + 150);
  await expect(page.locator('[data-settling]')).toHaveCount(0);
  const moved = (await sign.boundingBox())!;
  await page.reload();
  await expect(page.getByTestId('team-shift')).toBeVisible();
  const after = (await page.getByTestId('team-shift').boundingBox())!;
  expect(Math.abs(after.x - moved.x)).toBeLessThan(2);
  expect(Math.abs(after.y - moved.y)).toBeLessThan(2);

  await page.getByRole('complementary').getByRole('button', { name: 'wall_team' }).click();
  await page.getByRole('menuitemcheckbox', { name: 'wall_other' }).click();
  await page.getByTestId('nav-team').click();
  await expect(page.getByTestId('team-name')).toContainText('wall_other');
  const other = (await page.getByTestId('team-shift').boundingBox())!;
  expect(Math.abs(other.y - before.y)).toBeLessThan(2);
});

test('the team name stays centred on the board when the window is resized', async ({ page }) => {
  await rig({ op: 'create_workspace', workspace: 'centre_team' });
  await page.setViewportSize({ width: 1400, height: 1000 });
  await openTeam(page, 'centre_team');
  // Measured against the real board, not the label's own wrapper.
  const offset = async () => {
    const label = (await page.getByTestId('team-name').boundingBox())!;
    const board = (await page.locator('[data-layout="free"]').boundingBox())!;
    return label.x + label.width / 2 - (board.x + board.width / 2);
  };
  expect(Math.abs(await offset())).toBeLessThan(2);
  await page.setViewportSize({ width: 960, height: 800 });
  await expect.poll(async () => Math.abs(await offset())).toBeLessThan(2);
});
