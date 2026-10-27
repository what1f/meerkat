import { expect, type Page } from '@playwright/test';
import { test, rig } from './rig';
import { api, addFeishuMember, createFeishuTeam } from './feishu';

const open = async (page: Page, workspace: string, hash = 'members') => {
  await page.addInitScript((ws) => {
    localStorage.setItem('lankide.workspace', JSON.stringify(ws));
  }, workspace);
  await page.goto(`/#/${hash}`);
};

const save = async (page: Page) => {
  const key = page.getByTestId('member-save');
  await key.click();
  await expect(key).toBeDisabled();
};

// 1×1 PNG.
const png = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==',
  'base64',
);
const pngSrc = `data:image/png;base64,${png.toString('base64')}`;

const pickPng = async (page: Page) => {
  await page.getByRole('button', { name: /更换头像|Change avatar/ }).click();
  const chooser = page.waitForEvent('filechooser');
  await page.getByRole('button', { name: /^(上传|Upload)$/ }).click();
  await (await chooser).setFiles({ name: 'me.png', mimeType: 'image/png', buffer: png });
  await expect(page.getByTestId('avatar-picker')).toBeHidden();
};

// A member in a Feishu team, created and scanned through to the point where it can be opened.
const createdInFeishu = async (page: Page, workspace: string, avatar = false) => {
  await rig({ op: 'create_workspace', workspace, provider: 'feishu' });
  await rig({ op: 'feishu_scan', workspace, state: 'waiting' });
  await open(page, workspace, 'member/new');
  await page.getByTestId('member-name').fill('Ada');
  if (avatar) await pickPng(page);
  await page.getByTestId('member-create').click();
  await expect(page.getByTestId('member-create-qr')).toHaveAttribute('data-state', 'wait');
  await rig({ op: 'feishu_scan', workspace, state: 'approved' });
  const go = page.getByRole('button', { name: /^(打开|Open)$/ });
  await expect(go).toBeVisible();
  return go;
};

const stage = (page: Page) => page.locator('article', { has: page.getByTestId('member-name') });

test('B3a.1: a new member is created from the board and saved', async ({ page }) => {
  await rig({ op: 'create_workspace', workspace: 'members_create' });
  await open(page, 'members_create');
  await page.getByTestId('add-member').click();
  await expect(page.getByTestId('member-create')).toBeDisabled();
  // A new member already wears a built-in avatar.
  await expect(stage(page).locator('img:not([data-brand])')).toBeVisible();
  await page.getByTestId('member-name').fill('Ada');
  await page.getByTestId('member-create').click();
  await expect(page).toHaveURL(/#\/member\/m[0-9a-f]{12}$/);
  const id = page.url().split('/').pop()!;
  await expect(page.getByTestId('member-save')).toBeDisabled();
  await page.reload();
  await expect(page.getByTestId('member-name')).toHaveValue('Ada');
  await expect(stage(page).locator('img[src^="data:image/jpeg;base64,"]')).toBeVisible();
  await page.goto('/#/members');
  const card = page.getByTestId(`member-card-${id}`);
  await expect(card).toContainText('Ada');
  await expect(card.locator('img[src^="data:image/jpeg;base64,"]')).toBeVisible();
});

test('a member with unsaved edits is put back only once the edits are let go', async ({ page }) => {
  const member = { workspace: 'members_put_back', agent: 'hedy' };
  await rig({ op: 'create_workspace', workspace: member.workspace });
  await rig({ op: 'create_agent', ...member });
  await open(page, member.workspace, 'members');
  await page.getByTestId(`member-card-${member.agent}`).click();
  await page.getByTestId('member-name').fill('Hedy Lamarr');
  await page.keyboard.press('Escape');
  await expect(page).toHaveURL(new RegExp(`#/member/${member.agent}$`));
  await page
    .getByRole('status')
    .getByRole('button', { name: /放弃更改|Discard changes/ })
    .click();
  await expect(page).toHaveURL(/#\/members$/);
  await expect(page.getByTestId(`member-card-${member.agent}`)).not.toContainText('Hedy Lamarr');
});

test('B3a.2: a renamed member keeps its name and model after a reload', async ({ page }) => {
  const member = { workspace: 'members_edit', agent: 'grace' };
  await rig({ op: 'create_workspace', workspace: member.workspace });
  await rig({ op: 'create_agent', ...member });
  await open(page, member.workspace, `member/${member.agent}`);
  const name = page.getByTestId('member-name');
  await expect(name).not.toHaveValue('');
  const avatar = await page.request.post('/api/v1/get_agent_avatar', {
    data: { request_id: 'spec', agent: { workspace_id: member.workspace, agent_id: member.agent } },
  });
  expect(avatar.ok()).toBe(true);
  expect((await avatar.json()).avatar).toBeNull();
  await expect(
    page.getByText(/Some settings can’t be loaded right now|部分设置暂时加载不出来/),
  ).toHaveCount(0);
  await name.fill('Grace Hopper');
  await page.getByTestId('brain-picker').click();
  await page.getByRole('searchbox').fill('cobol-2');
  await page.getByRole('searchbox').press('Enter');
  await page.keyboard.press('Escape');
  await expect(page.getByTestId('brain-picker')).toContainText('cobol-2');
  await save(page);
  await page.reload();
  await expect(name).toHaveValue('Grace Hopper');
  await expect(page.getByTestId('brain-picker')).toContainText('cobol-2');
  await page.goto('/#/members');
  const card = page.getByTestId(`member-card-${member.agent}`);
  await expect(card).toContainText('Grace Hopper');
  await expect(card).toContainText('cobol-2');
});

test('a model shows its catalog display name on the board badge and the detail readout', async ({
  page,
}) => {
  const member = { workspace: 'members_model_name', agent: 'ada' };
  await rig({ op: 'create_workspace', workspace: member.workspace });
  await rig({ op: 'create_agent', ...member });
  await open(page, member.workspace, 'members');
  const card = page.getByTestId(`member-card-${member.agent}`);
  await expect(card).toContainText('Scripted');
  await expect(card).not.toContainText('scripted');
  await open(page, member.workspace, `member/${member.agent}`);
  await expect(page.getByTestId('brain-picker')).toContainText('Scripted');
  await expect(page.getByTestId('brain-picker')).not.toContainText('scripted');
});

test('saved badge names and roles keep their language-specific hierarchy', async ({ page }) => {
  const workspace = 'badge_typography';
  await rig({ op: 'create_workspace', workspace });
  await rig({ op: 'create_agent', workspace, agent: 'ada' });
  await open(page, workspace, 'member/ada');
  await page.getByTestId('member-name').fill('Alice Chen');
  for (const role of ['Product Designer', '产品经理']) {
    const roleField = page.getByPlaceholder(/^(Role|岗位)$/);
    await roleField.fill(role);
    await expect(roleField).toHaveCSS('font-weight', '600');
    await expect(roleField).toHaveCSS('text-transform', role === '产品经理' ? 'none' : 'uppercase');
    await expect(page.getByTestId('member-name')).toHaveCSS('font-size', '24.15px');
    await save(page);
    await page.goto('/#/members');
    const card = page.getByTestId('member-card-ada');
    await expect(card.locator('article header')).toHaveText('Alice Chen');
    await expect(card.locator('article header')).toHaveCSS('font-weight', '700');
    await expect(card.locator('article header')).toHaveCSS('font-family', /ui-rounded/);
    const line = card.locator('[data-latin]');
    await expect(line).toHaveText(role);
    await expect(line).toHaveCSS('font-weight', '600');
    await expect(line).toHaveCSS('text-transform', role === '产品经理' ? 'none' : 'uppercase');
    if (role === '产品经理') await expect(line).toHaveCSS('letter-spacing', 'normal');
    else
      expect(
        parseFloat(await line.evaluate((el) => getComputedStyle(el).letterSpacing)),
      ).toBeGreaterThan(1);
    await expect(card.locator('footer')).toHaveCSS('font-family', /ui-monospace/);
    await card.click();
    await expect(page.getByTestId('brain-picker').locator('[class*="model"]')).toHaveCSS(
      'font-family',
      /ui-monospace/,
    );
  }
});

test('B3a.3: a member paused by a failed run can be resumed', async ({ page }) => {
  const member = { workspace: 'members_pause', agent: 'linus' };
  await rig({ op: 'create_workspace', workspace: member.workspace });
  await rig({ op: 'create_agent', ...member });
  await open(page, member.workspace, `member/${member.agent}`);
  const pause = page.getByTestId('member-pause');
  await expect(pause).toBeDisabled();
  await page.getByRole('button', { name: /高级|Advanced/ }).click();
  const names = page.getByRole('textbox', { name: /^(名称|Variable)$/ });
  const values = page.getByRole('textbox', { name: /^(值|Value)$/ });
  const blank = (await names.count()) - 1;
  await names.nth(blank).fill('LANKIDE_SYSTEM_SCRIPT');
  await values.nth(blank).fill('{"behaviour":"fail_spawn","text":"model not found"}');
  await save(page);

  await rig({ op: 'mention', ...member, chat: 'group', person: 'alice', text: 'hello' });
  await expect(pause).toBeEnabled();
  await expect(pause).toHaveAttribute('data-lit', 'true');
  await expect(page.getByText('model not found').first()).toBeVisible();
  await page.goto('/#/members');
  await expect(
    page.getByTestId(`member-card-${member.agent}`).locator('[data-status="err"]'),
  ).toBeVisible();

  await page.getByTestId(`member-card-${member.agent}`).click();
  await expect(pause).toHaveAttribute('data-lit', 'true');
  await pause.click();
  await expect(pause).toBeDisabled();
  await expect(page.getByText('model not found')).toHaveCount(0);
  await page.goto('/#/members');
  await expect(
    page.getByTestId(`member-card-${member.agent}`).locator('[data-status="ok"]'),
  ).toBeVisible();
});

test('B3a.4: an uploaded avatar replaces the member picture', async ({ page }) => {
  const member = { workspace: 'members_avatar', agent: 'hedy' };
  await rig({ op: 'create_workspace', workspace: member.workspace });
  await rig({ op: 'create_agent', ...member });
  await open(page, member.workspace, `member/${member.agent}`);
  await page.getByRole('button', { name: /更换头像|Change avatar/ }).click();
  const picker = page.getByTestId('avatar-picker');
  await expect(picker).toBeVisible();
  const chooser = page.waitForEvent('filechooser');
  await page.getByRole('button', { name: /^(上传|Upload)$/ }).click();
  await (await chooser).setFiles({ name: 'me.png', mimeType: 'image/png', buffer: png });
  await expect(picker).toBeHidden();
  await save(page);
  await page.goto('/#/members');
  const src = pngSrc;
  await expect(
    page.getByTestId(`member-card-${member.agent}`).locator(`img[src="${src}"]`),
  ).toBeVisible();
});

test('B3a.5: deleting a member asks for confirmation', async ({ page }) => {
  const member = { workspace: 'members_delete', agent: 'alan' };
  await rig({ op: 'create_workspace', workspace: member.workspace });
  await rig({ op: 'create_agent', ...member });
  await open(page, member.workspace, `member/${member.agent}`);
  const dialog = page.getByTestId('confirm-delete-member');
  await page.getByTestId('member-delete').click();
  await expect(dialog).toBeVisible();
  await dialog.getByRole('button', { name: /^(取消|Cancel)$/ }).click();
  await expect(dialog).toBeHidden();
  await page.goto('/#/members');
  await expect(page.getByTestId(`member-card-${member.agent}`)).toBeVisible();

  await page.getByTestId(`member-card-${member.agent}`).click();
  await page.getByTestId('member-delete').click();
  await dialog.getByRole('button', { name: /^(删除|Delete)$/ }).click();
  await expect(page).toHaveURL(/#\/members$/);
  await expect(page.getByTestId(`member-card-${member.agent}`)).toHaveCount(0);
  await page.reload();
  await expect(page.getByTestId('add-member')).toBeVisible();
  await expect(page.getByTestId(`member-card-${member.agent}`)).toHaveCount(0);
});

test('B3a.6: a member that starts working lights up its card and console screen', async ({
  page,
}) => {
  const member = { workspace: 'members_busy', agent: 'barbara' };
  await rig({ op: 'create_workspace', workspace: member.workspace });
  await rig({ op: 'create_agent', ...member });
  await open(page, member.workspace);
  const card = page.getByTestId(`member-card-${member.agent}`);
  await expect(card.locator('[data-status="ok"]')).toBeVisible();

  await rig({ op: 'hold_next_run', ...member });
  await rig({ op: 'mention', ...member, chat: 'group', person: 'alice', text: 'work' });
  await rig({ op: 'wait_running', ...member });
  await expect(card.locator('[data-status="busy"]')).toBeVisible();
  await card.click();
  const screen = page.getByTestId('brain-picker');
  await expect(screen.locator('[data-status="busy"]')).toBeVisible();

  await rig({ op: 'release_run', ...member });
  await rig({ op: 'settle' });
  await expect(screen.locator('[data-status="ok"]')).toBeVisible();
  await page.goto('/#/members');
  await expect(card.locator('[data-status="ok"]')).toBeVisible();
});

test('B3a.8: launch arguments keep what is typed and quoted arguments keep their spaces', async ({
  page,
}) => {
  const member = { workspace: 'members_args', agent: 'margaret' };
  await rig({ op: 'create_workspace', workspace: member.workspace });
  await rig({ op: 'create_agent', ...member });
  await open(page, member.workspace, `member/${member.agent}`);
  await expect(page.getByTestId('member-name')).not.toHaveValue('');
  await page.getByRole('button', { name: /高级|Advanced/ }).click();
  const args = page.getByRole('textbox', { name: /^(启动参数|Launch arguments)$/ });
  const typed = '--foo bar "two words" ';
  await args.pressSequentially(typed);
  await expect(args).toHaveValue(typed);
  await save(page);
  await page.reload();
  await page.getByRole('button', { name: /高级|Advanced/ }).click();
  await expect(args).toHaveValue('--foo bar "two words"');
  const read = await page.request.post('/api/v1/get_agent', {
    data: { request_id: 'spec', agent: { workspace_id: member.workspace, agent_id: member.agent } },
  });
  expect((await read.json()).launch_arguments).toEqual(['--foo', 'bar', 'two words']);
});

test('B3a.7: a member created in a Feishu team waits for its app to be set up', async ({
  page,
}) => {
  const workspace = 'members_feishu';
  await rig({ op: 'create_workspace', workspace, provider: 'feishu' });
  await rig({ op: 'feishu_scan', workspace, state: 'waiting' });
  await open(page, workspace, 'member/new');
  await page.getByTestId('member-name').fill('Ada');
  await page.getByTestId('member-create').click();
  // The badge shows the member being created and the creation cannot be repeated.
  await expect(stage(page).locator('[aria-busy="true"]').first()).toBeVisible();
  await expect(page.getByTestId('member-create')).toBeDisabled();

  const qr = page.getByTestId('member-create-qr');
  await expect(qr).toHaveAttribute('data-state', 'wait');
  const registrationQr = await qr.locator('svg').innerHTML();
  await rig({ op: 'feishu_scan', workspace, state: 'complete' });
  await expect(qr).toBeVisible();
  await expect.poll(() => qr.locator('svg').innerHTML()).not.toBe(registrationQr);
  await expect(page.getByTestId('member-create-permissions')).toBeVisible();
  await page
    .context()
    .route('https://open.feishu.cn/page/launcher?**', (route) => route.fulfill({ body: 'Feishu' }));
  const opened = page.waitForEvent('popup');
  await page.getByTestId('member-create-permissions').click();
  const confirmation = await opened;
  await expect(confirmation).toHaveURL(
    /open\.feishu\.cn\/page\/launcher\?clientID=cli_registered_1&addons=/,
  );
  await confirmation.close();
  await page.screenshot({ path: test.info().outputPath('permissions.png') });
  await expect(page.getByText(/Waiting for approval|等待审批/)).toBeVisible();
  await expect(page.getByRole('button', { name: /^(打开|Open)$/ })).toHaveCount(0);
  await rig({ op: 'feishu_scan', workspace, state: 'approved' });
  await expect(qr).toBeHidden();

  await page.getByRole('button', { name: /^(打开|Open)$/ }).click();
  await expect(page).toHaveURL(/#\/member\/m[0-9a-f]{12}$/);
  const id = page.url().split('/').pop()!;
  // The built-in avatar chosen for the member went up when it was opened.
  await page.reload();
  await expect(stage(page).locator('img[src^="data:image/jpeg;base64,"]')).toBeVisible();
  await page.goto('/#/members');
  const card = page.getByTestId(`member-card-${id}`);
  await expect(card).toContainText('Ada');
  await expect(card.locator('img[src^="data:image/jpeg;base64,"]')).toBeVisible();
});

test('B3a.9: an avatar that fails to upload stays picked and opening can be tried again', async ({
  page,
}) => {
  const workspace = 'members_feishu_retry';
  let created = 0;
  let failNext = true;
  await page.route('**/api/v1/create_agent', (route) => {
    created += 1;
    return route.continue();
  });
  const go = await createdInFeishu(page, workspace, true);
  await page.route('**/api/v1/update_agent', (route) => {
    if (!failNext) return route.continue();
    failNext = false;
    return route.fulfill({
      status: 400,
      contentType: 'application/json',
      body: JSON.stringify({ code: 'failed_precondition', message: 'unavailable', detail: null }),
    });
  });
  await go.click();
  // The failure leaves the new member page as it was, still holding the picked avatar.
  await expect(page.getByText(/Something went wrong|没有成功/).first()).toBeVisible();
  await expect(page).toHaveURL(/#\/member\/new$/);
  await expect(stage(page).locator(`img[src="${pngSrc}"]`)).toBeVisible();
  await go.click();
  await expect(page).toHaveURL(/#\/member\/m[0-9a-f]{12}$/);
  const id = page.url().split('/').pop()!;
  await page.goto('/#/members');
  await expect(page.getByTestId(`member-card-${id}`).locator(`img[src="${pngSrc}"]`)).toBeVisible();
  expect(created).toBe(1);
});

test('B3a.10: opening a new member waits for its avatar and cannot be repeated', async ({
  page,
}) => {
  const workspace = 'members_feishu_once';
  const go = await createdInFeishu(page, workspace, true);
  let uploads = 0;
  let release = () => {};
  const held = new Promise<void>((resolve) => (release = resolve));
  await page.route('**/api/v1/update_agent', async (route) => {
    uploads += 1;
    await held;
    await route.continue();
  });
  await go.click();
  await expect(go).toBeDisabled();
  // A second press while the avatar is on its way does nothing and does not leave the page.
  await go.click({ force: true });
  await expect(page).toHaveURL(/#\/member\/new$/);
  release();
  await expect(page).toHaveURL(/#\/member\/m[0-9a-f]{12}$/);
  expect(uploads).toBe(1);
});

test('B3a.9: the advanced drawer edits variables, arguments and command path, then saves them', async ({
  page,
}) => {
  const member = { workspace: 'members_advanced', agent: 'barbara' };
  await rig({ op: 'create_workspace', workspace: member.workspace });
  await rig({ op: 'create_agent', ...member });
  await open(page, member.workspace, `member/${member.agent}`);
  await expect(page.getByTestId('member-name')).not.toHaveValue('');
  await page.getByRole('button', { name: /高级|Advanced/ }).click();
  const names = page.getByRole('textbox', { name: /^(名称|Variable)$/ });
  const values = page.getByRole('textbox', { name: /^(值|Value)$/ });
  const args = page.getByRole('textbox', { name: /^(启动参数|Launch arguments)$/ });
  const path = page.getByRole('textbox', { name: /^(命令行路径|Command path)$/ });
  // A filled row is always followed by one blank row.
  const rows = await names.count();
  await names.nth(rows - 1).fill('HTTPS_PROXY');
  await expect(names).toHaveCount(rows + 1);
  await values.nth(rows - 1).fill('http://127.0.0.1:7890');
  await args.fill('--verbose');
  await path.fill('/usr/local/bin/codex');
  await save(page);
  const read = await page.request.post('/api/v1/get_agent', {
    data: { request_id: 'spec', agent: { workspace_id: member.workspace, agent_id: member.agent } },
  });
  const saved = await read.json();
  expect(saved.environment_variables).toContainEqual({
    name: 'HTTPS_PROXY',
    value: 'http://127.0.0.1:7890',
  });
  expect(saved.launch_arguments).toEqual(['--verbose']);
  expect(saved.cli_path).toBe('/usr/local/bin/codex');
  await page.reload();
  await page.getByRole('button', { name: /高级|Advanced/ }).click();
  await expect(args).toHaveValue('--verbose');
  await expect(path).toHaveValue('/usr/local/bin/codex');
});

test('human badges show contact profiles and save positions without changing identity', async ({
  page,
  baseURL,
}) => {
  const workspace = 'human_badges';
  await createFeishuTeam(workspace);
  await addFeishuMember(baseURL!, workspace);
  await rig({ op: 'feishu_person', workspace, user: 'bob', name: 'Bob', tenant: 'tenant-a' });
  await addFeishuMember(baseURL!, workspace, 'beta');
  await open(page, workspace);
  const cards = page.getByTestId(/^human-card-/);
  await expect(cards).toHaveCount(2);
  const alice = cards.filter({ hasText: 'Alice' });
  const bob = cards.filter({ hasText: 'Bob' });
  await expect(alice).toContainText('Team Leader');
  await expect(alice.locator('footer')).toHaveCount(0);
  await expect(alice.locator('[data-status]')).toHaveCount(0);
  await expect
    .poll(() =>
      alice
        .locator('img:not([data-brand])')
        .evaluate((img: HTMLImageElement) => img.complete && img.naturalWidth > 0),
    )
    .toBe(true);
  await page.screenshot({ path: '/tmp/lankide-web/team-humans-board.png', animations: 'disabled' });
  await bob.click();
  await bob.getByRole('button', { name: /编辑岗位|Edit role/ }).click();
  const position = page.getByTestId('human-position');
  await expect(position).toHaveValue('');
  await expect(page.getByTestId('human-profile').locator('textarea')).toHaveCount(0);
  await position.fill('Draft');
  await position.press('Escape');
  await expect(position).toHaveValue('');
  await position.fill('Designer');
  await position.press('Enter');
  await expect
    .poll(
      async () =>
        (
          await api(baseURL!, 'list_human_members', { workspace: { workspace_id: workspace } })
        ).find((p: { name: string }) => p.name === 'Bob').position,
    )
    .toBe('Designer');
  await position.fill('Team Leader');
  await position.press('Tab');
  await expect
    .poll(
      async () =>
        (
          await api(baseURL!, 'list_human_members', { workspace: { workspace_id: workspace } })
        ).find((p: { name: string }) => p.name === 'Bob').position,
    )
    .toBe('Team Leader');
  await page.reload();
  await expect(position).toHaveValue('Team Leader');
  const roster = await api(baseURL!, 'list_human_members', {
    workspace: { workspace_id: workspace },
  });
  expect(roster.find((p: { name: string }) => p.name === 'Bob').team_role).toBe('member');
  await position.fill('x'.repeat(121));
  await position.press('Enter');
  await expect(position).toHaveValue('Team Leader');
  await page.screenshot({
    path: '/tmp/lankide-web/team-humans-profile.png',
    animations: 'disabled',
  });
  await page.goto('/#/members');
  await expect(cards).toHaveCount(2);
});

test('a creator without contact access still gets a human badge', async ({ page, baseURL }) => {
  const workspace = 'human_fallback';
  await createFeishuTeam(workspace);
  await rig({
    op: 'feishu_person',
    workspace,
    user: 'fallback',
    name: 'Hidden',
    tenant: 'tenant-a',
    contact_unavailable: true,
  });
  await addFeishuMember(baseURL!, workspace);
  await open(page, workspace);
  const card = page.getByTestId(/^human-card-/);
  await expect(card).toHaveCount(1);
  await expect(card).not.toContainText('Hidden');
  await expect
    .poll(() =>
      card
        .locator('img:not([data-brand])')
        .evaluate((img: HTMLImageElement) => img.complete && img.naturalWidth > 0),
    )
    .toBe(true);
});

for (const language of ['en', 'zh']) {
  test(`a creator from another organization is refused in ${language}`, async ({
    page,
    baseURL,
  }) => {
    const workspace = `human_enterprise_${language}`;
    await createFeishuTeam(workspace);
    await addFeishuMember(baseURL!, workspace);
    await rig({
      op: 'feishu_person',
      workspace,
      user: 'outsider',
      name: 'Outsider',
      tenant: 'other',
    });
    await rig({ op: 'feishu_scan', workspace, state: 'waiting' });
    await page.addInitScript((lang) => {
      localStorage.setItem('lankide.language', JSON.stringify(lang));
    }, language);
    await open(page, workspace, 'member/new');
    await page.getByTestId('member-name').fill('Second');
    await page.getByTestId('member-create').click();
    await expect(page.getByTestId('member-create-qr')).toHaveAttribute('data-state', 'wait');
    await rig({ op: 'feishu_scan', workspace, state: 'approved' });
    await expect(
      page.getByText(
        language === 'en'
          ? 'This person belongs to another organization.'
          : '这位成员来自其他企业，无法加入',
        { exact: true },
      ),
    ).toBeVisible();
    const people = await api(baseURL!, 'list_human_members', {
      workspace: { workspace_id: workspace },
    });
    expect(people).toHaveLength(1);
    expect(people[0].name).toBe('Alice');
  });
}

test('rapid human position saves keep their order and preserve a newer draft', async ({
  page,
  baseURL,
}) => {
  const workspace = 'human_rapid_saves';
  await createFeishuTeam(workspace);
  await addFeishuMember(baseURL!, workspace);
  await open(page, workspace);
  await page.getByTestId(/^human-card-/).click();
  await page.getByRole('button', { name: /编辑岗位|Edit role/ }).click();
  const position = page.getByTestId('human-position');
  const edits = async (values: string[]) => {
    // Dispatch normal input/focus/blur events in one browser turn. The real
    // management requests remain in flight while the next edit is made.
    await position.evaluate((input: HTMLInputElement, values) => {
      const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
      values.forEach((value, i) => {
        input.focus();
        set.call(input, value);
        input.dispatchEvent(new Event('input', { bubbles: true }));
        if (i < values.length - 1) input.blur();
      });
    }, values);
  };
  const saved = async () =>
    (
      await api(baseURL!, 'list_human_members', {
        workspace: { workspace_id: workspace },
      })
    )[0].position;
  await edits(['Designer', 'Team Leader', 'Staff Engineer']);
  await expect.poll(saved).toBe('Team Leader');
  await expect(position).toHaveValue('Staff Engineer');
  await position.press('Tab');
  await expect.poll(saved).toBe('Staff Engineer');
  await edits(['x'.repeat(121), 'Researcher', 'Newest draft']);
  await expect.poll(saved).toBe('Researcher');
  await expect(position).toHaveValue('Newest draft');
  await position.press('Escape');
  await expect(position).toHaveValue('Researcher');
  await page.reload();
  await expect(position).toHaveValue('Researcher');
});

for (const language of ['en', 'zh']) {
  test(`human badge flips, syncs profiles and preserves failed profiles in ${language}`, async ({
    page,
    baseURL,
  }) => {
    const workspace = `human_sync_${language}`;
    await createFeishuTeam(workspace);
    await addFeishuMember(baseURL!, workspace);
    await page.addInitScript(
      (lang) => localStorage.setItem('lankide.language', JSON.stringify(lang)),
      language,
    );
    await open(page, workspace);
    const card = page.getByTestId(/^human-card-/);
    const flip = card.locator('[data-flipped]');
    const sync = card.getByRole('button', { name: /同步资料|Sync profile/ });
    const role = card.getByRole('button', { name: /编辑岗位|Edit role/ });
    const oldPicture = await card.locator('img:not([data-brand])').getAttribute('src');
    await card.locator('article').click();
    await expect(flip).toHaveAttribute('data-flipped', 'true');
    await expect(page).toHaveURL(/#\/members$/);
    await expect(card.locator('[data-brand="feishu"]')).toBeVisible();
    await flip.focus();
    await page.keyboard.press('Enter');
    await expect(flip).toHaveAttribute('data-flipped', 'false');
    await page.keyboard.press('Space');
    await expect(flip).toHaveAttribute('data-flipped', 'true');
    await rig({
      op: 'feishu_profile',
      workspace,
      name: 'Updated Alice',
      signature: 'Keep it simple',
      avatar_changed: true,
      delay_ms: 500,
    });
    await sync.click();
    await expect(sync).toHaveAttribute('aria-busy', 'true');
    await sync.click({ force: true });
    await expect(flip).toHaveAttribute('data-flipped', 'false');
    await expect(card).toContainText('Updated Alice');
    await expect(card.getByTestId('badge-bio')).toHaveText('Keep it simple');
    const newPicture = await card.locator('img:not([data-brand])').getAttribute('src');
    expect(newPicture).not.toBe(oldPicture);
    await expect
      .poll(() =>
        card
          .locator('img:not([data-brand])')
          .evaluate((img: HTMLImageElement) => img.complete && img.naturalWidth > 0),
      )
      .toBe(true);
    // A second successful manual sync is inside the automatic throttle window.
    await card.locator('article').click();
    await rig({
      op: 'feishu_profile',
      workspace,
      name: 'Manual Alice',
      signature: 'Manual signature',
      avatar_changed: true,
    });
    await sync.click();
    await expect(flip).toHaveAttribute('data-flipped', 'false');
    await expect(card).toContainText('Manual Alice');
    await expect(card.getByTestId('badge-bio')).toHaveText('Manual signature');
    await expect(card.locator('img:not([data-brand])')).toHaveAttribute('src', newPicture!);
    await card.locator('article').click();
    await rig({ op: 'feishu_profile', workspace, name: 'Failed Alice', unavailable: true });
    await sync.click();
    await expect(
      page.getByText(language === 'en' ? 'Something went wrong. Try again.' : '没有成功，请重试', {
        exact: true,
      }),
    ).toBeVisible();
    await expect(flip).toHaveAttribute('data-flipped', 'true');
    const roster = await api(baseURL!, 'list_human_members', {
      workspace: { workspace_id: workspace },
    });
    expect(roster[0].name).toBe('Manual Alice');
    await page.screenshot({
      path: `/tmp/lankide-web/human-sync-back-${language}.png`,
      animations: 'disabled',
    });
    await role.click();
    await expect(page.getByTestId('human-position')).toBeVisible();
    const profile = page.getByTestId('human-profile');
    await expect(profile).toContainText('Manual Alice');
    const signature = profile.getByTestId('badge-bio');
    await expect(signature).toHaveText('Manual signature');
    await signature.click();
    await expect(signature.locator('input, textarea, [contenteditable="true"]')).toHaveCount(0);
    await expect(profile.getByRole('textbox')).toHaveCount(1);
    await page.reload();
    await expect(page.getByTestId('human-profile').getByTestId('badge-bio')).toHaveText(
      'Manual signature',
    );
  });
}

test('a message refreshes the human badge while the bot replies', async ({ page, baseURL }) => {
  const workspace = 'human_sync_inbound';
  await createFeishuTeam(workspace);
  await addFeishuMember(baseURL!, workspace);
  await open(page, workspace);
  const card = page.getByTestId(/^human-card-/);
  await expect(card).toContainText('Alice');
  const picture = await card.locator('img:not([data-brand])').getAttribute('src');
  await rig({
    op: 'feishu_profile',
    workspace,
    name: 'Message Alice',
    signature: 'Message signature',
    avatar_changed: true,
  });
  await rig({ op: 'dm', workspace, agent: 'alpha', text: 'hello' });
  await expect(card).toContainText('Message Alice');
  await expect(card.getByTestId('badge-bio')).toHaveText('Message signature');
  await expect(card.locator('img:not([data-brand])')).not.toHaveAttribute('src', picture!);
  await expect
    .poll(
      async () =>
        (await rig({ op: 'settle' })).outbound?.filter((v) => v.workspace === workspace).length ??
        0,
    )
    .toBeGreaterThan(0);
  await rig({
    op: 'feishu_profile',
    workspace,
    name: 'Missing signature Alice',
    advance_hour: true,
  });
  await rig({ op: 'dm', workspace, agent: 'alpha', text: 'keep the missing signature' });
  await expect(card).toContainText('Missing signature Alice');
  await rig({ op: 'settle' });
  await expect(card.getByTestId('badge-bio')).toHaveText('Message signature');
  await rig({
    op: 'feishu_profile',
    workspace,
    name: 'Cleared after message',
    signature: '',
    advance_hour: true,
  });
  await rig({ op: 'dm', workspace, agent: 'alpha', text: 'clear the signature' });
  await expect(card).toContainText('Cleared after message');
  await expect(card.getByTestId('badge-bio')).toHaveCount(0);
});

test('human signatures preserve missing fields and clear on an empty value', async ({
  page,
  baseURL,
}) => {
  const workspace = 'human_signature_presence';
  await createFeishuTeam(workspace);
  await rig({
    op: 'feishu_profile',
    workspace,
    name: 'Alice',
    signature: 'Created with a signature',
  });
  await addFeishuMember(baseURL!, workspace);
  await open(page, workspace);
  const card = page.getByTestId(/^human-card-/);
  await expect(card.getByTestId('badge-bio')).toHaveText('Created with a signature');
  const sync = async () => {
    await card.locator('article').click();
    await card.getByRole('button', { name: /同步资料|Sync profile/ }).click();
    await expect(card.locator('[data-flipped]')).toHaveAttribute('data-flipped', 'false');
  };
  // A missing field and a denied field both preserve the last known signature.
  await rig({ op: 'feishu_profile', workspace, name: 'Missing field' });
  await sync();
  await expect(card).toContainText('Missing field');
  await expect(card.getByTestId('badge-bio')).toHaveText('Created with a signature');
  await rig({
    op: 'feishu_profile',
    workspace,
    name: 'Permission pending',
    signature: 'Unavailable',
    signature_unavailable: true,
  });
  await sync();
  await expect(card).toContainText('Permission pending');
  await expect(card.getByTestId('badge-bio')).toHaveText('Created with a signature');
  await rig({ op: 'feishu_profile', workspace, name: 'Cleared signature', signature: '' });
  await sync();
  await expect(card).toContainText('Cleared signature');
  await expect(card.getByTestId('badge-bio')).toHaveCount(0);
  await card.locator('article').click();
  await card.getByRole('button', { name: /编辑岗位|Edit role/ }).click();
  await expect(page.getByTestId('human-profile').getByTestId('badge-bio')).toHaveCount(0);
});
