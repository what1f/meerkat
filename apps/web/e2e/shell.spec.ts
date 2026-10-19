import { expect } from '@playwright/test';
import { test, rig } from './rig';

test('B4.1: navigate and receive a new workspace without a page reload', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
  // A team exists before the page opens; teams made later must not replace it.
  await rig({ op: 'create_workspace', workspace: 'team_home' });
  const eventsReady = page.waitForEvent('websocket', (socket) =>
    socket.url().includes('/api/v1/events'),
  );
  await page.goto('/');
  const events = await eventsReady;
  // A received frame proves the subscription is active before the mutation.
  const frame = events.waitForEvent('framereceived');
  for (const name of ['members', 'team', 'office', 'members']) {
    const item = page.getByTestId(`nav-${name}`);
    await item.click();
    await expect(page).toHaveURL(new RegExp(`#/${name}$`));
    await expect(item).toHaveAttribute('aria-current', 'page');
  }
  const document = await page.locator('html').elementHandle();
  const switcher = page.getByTestId('team-switcher');
  const menu = page.getByRole('menu');
  await expect(switcher).toHaveText(/\S/);
  const current = (await switcher.textContent())!.trim();
  await rig({ op: 'create_workspace', workspace: 'browser_team' });
  await frame;
  // The team title opens a menu: once the new team is listed, the current team is unchanged.
  await switcher.click();
  await expect(menu.getByRole('menuitemcheckbox', { name: 'browser_team' })).toBeVisible();
  await expect(menu.getByRole('menuitemcheckbox', { checked: true })).toHaveText(current);
  await expect(switcher).toHaveText(current);
  await menu.getByRole('menuitemcheckbox', { name: 'browser_team' }).click();
  await expect(menu).toBeHidden();
  await expect(switcher).toHaveText('browser_team');
  await expect(page).toHaveURL(/#\/members$/);
  await switcher.click();
  await expect(menu.getByRole('menuitemcheckbox', { checked: true })).toHaveText('browser_team');
  await page.keyboard.press('Escape');
  await expect(menu).toBeHidden();
  expect(await document!.evaluate((node) => node === window.document.documentElement)).toBe(true);
  expect(errors).toEqual([]);
});

test('mock group mentions run on the Machine and held runs can be released twice', async () => {
  const member = { workspace: 'harness_team', agent: 'bruce' };
  await rig({ op: 'create_workspace', workspace: member.workspace });
  await rig({ op: 'create_agent', ...member });
  for (let run = 0; run < 2; run++) {
    await rig({ op: 'hold_next_run', ...member });
    await rig({ op: 'mention', ...member, chat: 'group', person: 'alice', text: `hello ${run}` });
    await rig({ op: 'wait_running', ...member });
    await rig({ op: 'release_run', ...member });
    const result = await rig({ op: 'settle' });
    const replies = result.outbound!.filter(
      (message) => message.workspace === member.workspace && message.chat === 'group',
    );
    expect(replies).toHaveLength(run + 1);
    expect(replies.at(-1)).toMatchObject({ account: 'bot-bruce', content: 'hi' });
  }
});

test('an empty team shows one centered sleeve and opens member creation', async ({ page }) => {
  await rig({ op: 'create_workspace', workspace: 'empty_sleeve' });
  await page.addInitScript(() =>
    localStorage.setItem('lankide.workspace', JSON.stringify('empty_sleeve')),
  );
  await page.goto('/#/members');
  const board = page.locator('main [data-layout="rails"]');
  const sleeve = page.getByTestId('add-member');
  await expect(sleeve).toBeVisible();
  await expect(board.locator('article')).toHaveCount(0);
  await expect(board.getByRole('button')).toHaveCount(1);
  await expect(board.locator('[data-peg]')).toHaveCount(1);
  const box = (await sleeve.boundingBox())!;
  expect(box.x + box.width / 2).toBeCloseTo(
    (await page.locator('main').boundingBox())!.width / 2,
    1,
  );
  await sleeve.click();
  await expect(page).toHaveURL(/#\/member\/new$/);
  await expect(page.getByTestId('member-name')).toBeVisible();
});

test('the team menu opens settings and the dock uses localized icon navigation', async ({
  page,
}) => {
  await rig({ op: 'create_workspace', workspace: 'dock_menu' });
  for (let i = 0; i < 30; i++) await rig({ op: 'create_workspace', workspace: `menu_team_${i}` });
  await page.addInitScript(() => {
    if (!localStorage.getItem('lankide.language'))
      localStorage.setItem('lankide.language', JSON.stringify('en'));
  });
  await page.goto('/#/members');
  for (const [id, label] of [
    ['members', 'Members'],
    ['team', 'Team'],
    ['office', 'Office'],
  ]) {
    const item = page.getByTestId(`nav-${id}`);
    await expect(item).toHaveAccessibleName(label);
    await expect(item).toHaveText('');
    await item.hover();
    await expect(page.getByRole('tooltip', { name: label, exact: true })).toBeVisible();
  }
  await page.getByTestId('team-switcher').click();
  await expect(page.getByRole('menuitem', { name: 'New team', exact: true })).toBeVisible();
  await expect(page.getByTestId('team-settings')).toHaveText('Settings');
  await page.getByTestId('team-settings').click();
  await expect(page).toHaveURL(/#\/settings$/);
  await page.getByTestId('settings-language').click();
  await page.getByRole('option', { name: '简体中文' }).click();
  await page.getByTestId('team-switcher').click();
  await expect(page.getByTestId('team-settings')).toHaveText('设置');
  await expect(page.getByRole('menuitem', { name: '新建团队', exact: true })).toBeVisible();
  await page.getByTestId('team-settings').click();
  await page.reload();
  await expect(page.getByTestId('settings-language')).toHaveText('简体中文');
});
