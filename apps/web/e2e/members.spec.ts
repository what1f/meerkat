import { expect, type Page } from '@playwright/test';
import { test, rig } from './rig';

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
  await rig({ op: 'feishu_scan', workspace, state: 'complete' });
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

test('B3a.2: a renamed member keeps its name and model after a reload', async ({ page }) => {
  const member = { workspace: 'members_edit', agent: 'grace' };
  await rig({ op: 'create_workspace', workspace: member.workspace });
  await rig({ op: 'create_agent', ...member });
  await open(page, member.workspace, `member/${member.agent}`);
  const name = page.getByTestId('member-name');
  await expect(name).not.toHaveValue('');
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
  await rig({ op: 'feishu_scan', workspace, state: 'scanned' });
  await expect(qr).toHaveAttribute('data-state', 'scanned');
  await rig({ op: 'feishu_scan', workspace, state: 'complete' });
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

test('a profile picture that failed once loads again when it comes back', async ({ page }) => {
  const workspace = 'members_avatar_back';
  await rig({ op: 'create_workspace', workspace, provider: 'feishu' });
  let refreshed = 0;
  let reads = 0;
  let firstRequests = 0;
  await page.route('**/api/v1/invoke_provider_interface', async (route) => {
    const input = route.request().postDataJSON();
    if (input.interface_id !== 'feishu.workspace.owner') return route.continue();
    if (JSON.parse(input.input_json).action === 'refresh') refreshed += 1;
    else reads += 1;
    await route.fulfill({
      json: {
        output_json: JSON.stringify({
          state: 'ready',
          name: 'Owner',
          avatar: { resource_id: refreshed === 1 ? 99102 : 99101 },
        }),
      },
    });
  });
  await page.route('**/api/v1/resources/9910*', async (route) => {
    if (new URL(route.request().url()).pathname.endsWith('/99101') && ++firstRequests === 1)
      return route.abort('failed');
    return route.fulfill({ contentType: 'image/png', body: png });
  });
  await open(page, workspace);
  const card = page.getByTestId('lead-card');
  const avatar = card.locator('article img:not([data-brand])');
  const decoded = () =>
    avatar.evaluate((img: HTMLImageElement) => img.complete && img.naturalWidth > 0);
  await expect.poll(() => firstRequests).toBe(1);
  // The picture that cannot load gives way to a built-in avatar.
  await expect(avatar).toHaveAttribute('src', /assets\/\d\d-.*\.jpg$/);
  await expect.poll(decoded).toBe(true);
  await card.locator('[data-flipped]').press('Enter');
  const sync = card.getByRole('button', { name: /Sync|同步/ });
  await sync.click();
  await expect(avatar).toHaveAttribute('src', /resources\/99102/);
  await expect.poll(decoded).toBe(true);
  const before = reads;
  await sync.click();
  await expect.poll(() => reads).toBeGreaterThan(before);
  await expect(avatar).toHaveAttribute('src', /resources\/99101/);
  await expect.poll(decoded).toBe(true);
  expect(firstRequests).toBe(2);
});
