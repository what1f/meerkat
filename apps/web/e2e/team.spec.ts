import { expect, request, type Page } from '@playwright/test';
import { test, rig } from './rig';
import { api, addFeishuMember, createFeishuTeam } from './feishu';

const workspaceOf = (baseURL: string, workspace_id: string) =>
  api(baseURL, 'get_workspace', { workspace: { workspace_id } });

const tasksEnabled = async (baseURL: string, workspace: string) =>
  JSON.parse((await workspaceOf(baseURL, workspace)).provider_config_json).task_system_enabled;

async function openTeam(page: Page, workspace: string) {
  await page.addInitScript((id) => {
    localStorage.setItem('meerkat.workspace', JSON.stringify(id));
  }, workspace);
  await page.goto('/#/team');
  await expect(page.getByTestId('team-name')).toContainText(workspace);
}

// The wall shows a small pad; the full clipboard opens over it.
const board = (page: Page) => page.getByTestId('tasklist-board');
async function openList(page: Page) {
  const pad = page.getByTestId('tasklist');
  await expect(pad).toHaveAttribute('aria-haspopup', 'dialog');
  await pad.click();
  await expect(board(page)).toBeVisible();
  return board(page);
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

test('the team name is renamed in place and the dock follows', async ({ page, baseURL }) => {
  await rig({ op: 'create_workspace', workspace: 'rename_team' });
  await openTeam(page, 'rename_team');
  await page.getByTestId('team-name').getByRole('button').click();
  await page.getByTestId('team-name').getByRole('textbox').fill('Night Desk');
  await page.keyboard.press('Enter');
  await expect(page.getByTestId('team-switcher')).toBeVisible();
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

test('the Feishu task list is read-only and can be turned off', async ({ page, baseURL }) => {
  const workspace = 'tasks_team';
  await createFeishuTeam(workspace);
  await addFeishuMember(baseURL!, workspace);
  await turnTasksOn(baseURL!, workspace);
  await openTeam(page, workspace);
  await expect(page.getByTestId('tasklist').getByText('Draft the launch notes')).toBeVisible();
  const list = await openList(page);
  await expect(list.getByText('Draft the launch notes')).toBeVisible();
  await expect(list.getByRole('textbox')).toHaveCount(1);
  await expect(list.getByRole('checkbox').first()).toBeDisabled();
  const check = list.getByRole('checkbox').first();
  await expect(check).toHaveCSS('cursor', 'default');
  const border = await check.locator('..').evaluate((el) => getComputedStyle(el).borderColor);
  await check.hover({ force: true });
  await expect(check.locator('..')).toHaveCSS('border-color', border);
  await page.screenshot({ path: test.info().outputPath('read-only-tasklist.png') });
  for (const action of ['create', 'set_completed']) {
    const client = await request.newContext({ baseURL });
    const response = await client.post('/api/v1/invoke_provider_interface', {
      data: {
        request_id: action,
        workspace: { workspace_id: workspace },
        interface_id: 'feishu.workspace.tasks',
        input_json: JSON.stringify({
          action,
          text: 'Forbidden',
          id: 'remote-task-0',
          completed: true,
        }),
      },
    });
    expect(response.ok()).toBe(false);
    await client.dispose();
  }
  await page.reload();
  await openList(page);
  await expect(list.getByText('Draft the launch notes')).toBeVisible();
  await list.getByRole('button', { name: 'More' }).click();
  await page.getByRole('menuitem', { name: 'Turn off' }).click();
  await expect(list).toBeHidden();
  expect(await tasksEnabled(baseURL!, workspace)).toBe(false);
});

test('the task list name saves on Enter or blur and restores on failure or Escape', async ({
  page,
  baseURL,
}) => {
  const workspace = 'rename_tasks';
  await createFeishuTeam(workspace);
  await addFeishuMember(baseURL!, workspace);
  await turnTasksOn(baseURL!, workspace);
  await openTeam(page, workspace);
  await openList(page);
  const title = board(page).getByRole('textbox');
  await expect(title).toHaveValue(/.+/);
  const original = await title.inputValue();
  await title.fill('Cancelled');
  await title.press('Escape');
  await expect(title).toHaveValue(original);
  await title.fill('   ');
  await title.press('Enter');
  await expect(title).toHaveValue(original);
  await title.focus();
  await title.blur();
  expect((await rig({ op: 'feishu_tasklist', workspace, fail: false })).renames).toEqual([]);
  await title.fill('Release checklist');
  await title.press('Enter');
  await expect
    .poll(async () => (await rig({ op: 'feishu_tasklist', workspace, fail: false })).renames)
    .toEqual([
      [
        'Bearer tenant-cli_registered_1',
        {
          tasklist: { name: 'Release checklist' },
          update_fields: ['name'],
        },
      ],
    ]);
  await expect(title).toHaveAttribute('aria-busy', 'false');
  await expect(title).toHaveValue('Release checklist');
  await page.reload();
  await openList(page);
  await expect(title).toHaveValue('Release checklist');
  await title.fill('Launch tasks');
  await title.blur();
  await expect
    .poll(
      async () =>
        (await rig({ op: 'feishu_tasklist', workspace, fail: false })).renames?.length ?? 0,
    )
    .toBe(2);
  await expect(title).toHaveAttribute('aria-busy', 'false');
  await page.reload();
  await openList(page);
  await expect(title).toHaveValue('Launch tasks');
  await rig({ op: 'feishu_tasklist', workspace, fail: true });
  await title.fill('Denied');
  await title.press('Enter');
  await expect(title).toHaveValue('Launch tasks');
  await expect(page.getByRole('status')).toContainText('Couldn’t save changes');
  await page.reload();
  await openList(page);
  await expect(title).toHaveValue('Launch tasks');
});

test('a task list turned on before the team lead is bound can be retried or turned off', async ({
  page,
  baseURL,
}) => {
  const workspace = 'unbound_team';
  await createFeishuTeam(workspace);
  const pad = page.getByTestId('tasklist');
  const list = board(page);
  const retry = list.getByRole('button', { name: 'Try again' });
  const hint = 'The task list can’t be opened right now';
  let reads = 0;
  page.on('request', (request) => {
    if (request.url().endsWith('/api/v1/invoke_provider_interface')) {
      const body = request.postDataJSON() as { interface_id: string };
      if (body.interface_id === 'feishu.workspace.tasks') reads++;
    }
  });
  const taskRead = () =>
    page.waitForResponse(
      (response) =>
        response.url().endsWith('/api/v1/invoke_provider_interface') &&
        (response.request().postDataJSON() as { interface_id: string }).interface_id ===
          'feishu.workspace.tasks',
    );
  const initialRead = taskRead();
  await openTeam(page, workspace);
  await initialRead;

  await pad.click();
  await expect.poll(() => tasksEnabled(baseURL!, workspace)).toBe(true);
  await openList(page);
  await expect(list).toContainText(hint, { timeout: 2_000 });
  await expect(retry).toBeVisible();
  await list.getByRole('button', { name: 'More' }).click();
  await page.getByRole('menuitem', { name: 'Turn off' }).click();
  await expect(list).toBeHidden();
  await expect(pad).toHaveText('Task list');
  expect(await tasksEnabled(baseURL!, workspace)).toBe(false);

  await pad.click();
  await expect.poll(() => tasksEnabled(baseURL!, workspace)).toBe(true);
  await openList(page);
  await expect(retry).toBeVisible();
  // After a reload nothing was ever read; retrying keeps the clipboard and its way to turn off.
  const beforeReload = reads;
  await page.reload();
  await openList(page);
  await expect(retry).toBeVisible({ timeout: 2_000 });
  expect(reads).toBe(beforeReload + 1); // One read, without automatic retries.
  const beforeRetry = reads;
  const retried = taskRead();
  await retry.click();
  await retried;
  await expect(list.getByRole('button', { name: 'More' })).toBeVisible();
  await expect(retry).toHaveAttribute('aria-busy', 'false');
  await expect(list).toContainText(hint);
  expect(reads).toBe(beforeRetry + 1);
  await expect(list).not.toContainText('Add member');
  await page.screenshot({ path: test.info().outputPath('task-unavailable.png') });
  await page.keyboard.press('Escape');
  await expect(list).toBeHidden();
  await page.getByTestId('team-switcher').click();
  await page.getByTestId('team-settings').click();
  await page.getByTestId('settings-language').click();
  await page.getByRole('option', { name: '简体中文' }).click();
  await page.getByTestId('nav-team').click();
  await openList(page);
  await expect(list).toContainText('任务清单暂时打不开');
  await expect(list).not.toContainText('添加成员');
  await page.screenshot({ path: test.info().outputPath('task-unavailable-zh.png') });
  // Once the team lead is bound the list comes back on its own.
  await addFeishuMember(baseURL!, workspace);
  await rig({ op: 'feishu_task', workspace, text: 'Book the venue' });
  await expect(list.getByText('Book the venue')).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(pad.getByText('Book the venue')).toBeVisible();
});

test('pieces moved on the wall stay where they were hung, per team', async ({ page }) => {
  await rig({ op: 'create_workspace', workspace: 'wall_team' });
  await rig({ op: 'create_workspace', workspace: 'wall_other' });
  await openTeam(page, 'wall_team');
  const sign = page.getByTestId('team-shift');
  const before = (await sign.boundingBox())!;
  await page.mouse.move(before.x + before.width / 2, before.y + 4);
  await page.mouse.down();
  // Clear of the timer schedule hung under the sign.
  await page.mouse.move(before.x + before.width / 2 + 200, before.y + 200, { steps: 12 });
  await page.mouse.up();
  await expect.poll(async () => (await sign.boundingBox())!.y).toBeGreaterThan(before.y + 150);
  await expect(page.locator('[data-settling]')).toHaveCount(0);
  const moved = (await sign.boundingBox())!;
  await page.reload();
  await expect(page.getByTestId('team-shift')).toBeVisible();
  const after = (await page.getByTestId('team-shift').boundingBox())!;
  expect(Math.abs(after.x - moved.x)).toBeLessThan(2);
  expect(Math.abs(after.y - moved.y)).toBeLessThan(2);

  await page.getByTestId('team-switcher').click();
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

test('the team name opens its editor from the top edge and from the keyboard', async ({ page }) => {
  await rig({ op: 'create_workspace', workspace: 'edge_team' });
  await openTeam(page, 'edge_team');
  const name = page.getByTestId('team-name').getByRole('button');
  const editor = page.getByTestId('team-name').getByRole('textbox');
  // The transparent top bar must not cover the board under it.
  await name.click({ position: { x: 20, y: 2 } });
  await expect(editor).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(editor).toHaveCount(0);
  await name.focus();
  await page.keyboard.press('Enter');
  await expect(editor).toBeFocused();
});

async function turnTasksOn(baseURL: string, workspace: string) {
  await api(baseURL, 'update_workspace', {
    workspace: { workspace_id: workspace },
    name: null,
    rules: null,
    provider_config_json: JSON.stringify({ task_system_enabled: true }),
    cover_resource_id: null,
    provider_secrets_json: null,
  });
  await addFeishuMember(baseURL, workspace, 'beta');
  for (const text of ['Draft the launch notes', 'Book the venue with the long name for the team'])
    await rig({ op: 'feishu_task', workspace, text });
}

test('a team with the task list on shows it as soon as the wall opens', async ({
  page,
  baseURL,
}) => {
  const workspace = 'listed_team';
  await createFeishuTeam(workspace);
  await addFeishuMember(baseURL!, workspace);
  await turnTasksOn(baseURL!, workspace);
  // Feishu is slow to answer; the clipboard must hang on the wall before it does.
  let release = () => {};
  const held = new Promise<void>((resolve) => (release = resolve));
  await page.route('**/api/v1/invoke_provider_interface', async (route) => {
    if ((route.request().postData() ?? '').includes('feishu.workspace.tasks')) await held;
    await route.continue();
  });
  await page.setViewportSize({ width: 1600, height: 1000 });
  await openTeam(page, workspace);

  const list = page.getByTestId('tasklist');
  await expect(list).toBeVisible();
  await expect(list.getByText('Draft the launch notes')).toHaveCount(0);
  release();
  await expect(list.getByText('Draft the launch notes')).toBeVisible();
  const view = page.viewportSize()!;
  const box = (await list.boundingBox())!;
  expect(box.x).toBeGreaterThanOrEqual(0);
  expect(box.x + box.width).toBeLessThanOrEqual(view.width);
  expect(box.y + box.height).toBeLessThanOrEqual(view.height);
  const rules = (await page.getByTestId('team-rules').boundingBox())!;
  expect(box.x >= rules.x + rules.width || box.y >= rules.y + rules.height).toBe(true);
});

test('dragging a piece leaves the wall texture and the board size alone', async ({
  page,
  baseURL,
}) => {
  const workspace = 'steady_team';
  await createFeishuTeam(workspace);
  await addFeishuMember(baseURL!, workspace);
  await turnTasksOn(baseURL!, workspace);
  await page.setViewportSize({ width: 1600, height: 1000 });
  await openTeam(page, workspace);
  await expect(page.getByTestId('tasklist').getByText('Draft the launch notes')).toBeVisible();

  const wall = () =>
    page.locator('[data-layout="free"]').evaluate((board) => {
      const holes = getComputedStyle(board, '::before').backgroundPosition;
      const scroll: number[] = [];
      for (let el: Element | null = board; el; el = el.parentElement)
        scroll.push(el.scrollHeight, el.scrollWidth);
      return { holes, scroll };
    });
  await page.waitForTimeout(500);
  const still = await wall();
  // Held by the cord above the paper, where no field takes the pointer.
  const piece = page.locator('[data-spot="rules"]');
  const at = (await piece.boundingBox())!;
  await page.mouse.move(at.x + at.width / 2, at.y + 4);
  await page.mouse.down();
  for (const [dx, dy] of [
    [40, 80],
    [90, 150],
    [-30, 250],
  ]) {
    await page.mouse.move(at.x + at.width / 2 + dx, at.y + 4 + dy, { steps: 8 });
    // Something redraws the wall mid-drag: a task arrives from Feishu.
    await rig({ op: 'feishu_task', workspace, text: `Arrived ${dy}` });
    const arrived = page.waitForResponse(
      async (response) =>
        response.url().endsWith('/api/v1/invoke_provider_interface') &&
        (await response.text()).includes(`Arrived ${dy}`),
    );
    await page.evaluate(() => window.dispatchEvent(new Event('focus')));
    await arrived;
    await page.waitForTimeout(100);
    expect(await wall()).toEqual(still);
  }
  await page.mouse.up();
});

test('with the task list off the rules hang on the middle hole of the board', async ({
  page,
  baseURL,
}) => {
  const workspace = 'bare_team';
  await createFeishuTeam(workspace);
  await addFeishuMember(baseURL!, workspace);
  await page.setViewportSize({ width: 1600, height: 1000 });
  await openTeam(page, workspace);
  const boxes = async () => {
    const board = (await page.locator('[data-layout="free"]').boundingBox())!;
    const rules = (await page.getByTestId('team-rules').boundingBox())!;
    const list = (await page.getByTestId('tasklist').boundingBox())!;
    const shift = (await page.getByTestId('team-shift').boundingBox())!;
    return { board, rules, list, shift };
  };
  const apart = (a: { x: number; y: number; width: number; height: number }, b: typeof a) =>
    a.x >= b.x + b.width || b.x >= a.x + a.width || a.y >= b.y + b.height || b.y >= a.y + a.height;
  await expect(page.getByTestId('tasklist')).toBeVisible();
  // The full-width board can have an even number of 24px holes. Its two middle
  // pegs then sit 12px from the center; the rules must hang on one of those pegs.
  await expect
    .poll(async () => {
      const { board, rules } = await boxes();
      return Math.abs(rules.x + rules.width / 2 - (board.x + board.width / 2));
    })
    .toBeLessThanOrEqual(12);
  const peg = (await page
    .getByTestId('team-rules')
    .locator('xpath=ancestor::section[1]')
    .locator('[data-peg]')
    .boundingBox())!;
  const centeredRules = (await boxes()).rules;
  expect(peg.x + peg.width / 2).toBeCloseTo(centeredRules.x + centeredRules.width / 2, 1);
  const bare = await boxes();
  expect(apart(bare.rules, bare.list)).toBe(true);
  expect(apart(bare.rules, bare.shift)).toBe(true);
  expect(apart(bare.list, bare.shift)).toBe(true);

  await page.getByTestId('tasklist').click();
  await expect(page.getByTestId('tasklist')).toHaveAttribute('aria-haspopup', 'dialog');
  await expect
    .poll(async () => {
      const { board, rules } = await boxes();
      return board.x + board.width / 2 - (rules.x + rules.width / 2);
    })
    .toBeGreaterThan(100);
  const listed = await boxes();
  expect(apart(listed.rules, listed.list)).toBe(true);
  expect(apart(listed.rules, listed.shift)).toBe(true);
  expect(apart(listed.list, listed.shift)).toBe(true);
  expect(listed.list.x).toBeGreaterThan(listed.rules.x);
});
