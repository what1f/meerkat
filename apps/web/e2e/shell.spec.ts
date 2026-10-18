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
  for (const name of ['members', 'team', 'office', 'settings', 'members']) {
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

test('the sidebar edge resizes, folds to icons, restores on double click and persists', async ({
  page,
}) => {
  await rig({ op: 'create_workspace', workspace: 'sidebar_team' });
  await page.addInitScript(() => localStorage.setItem('lankide.language', JSON.stringify('en')));
  await page.goto('/#/members');
  const side = page.locator('aside');
  const grip = page.getByRole('separator');
  const width = async () => Math.round((await side.boundingBox())!.width);
  const drag = async (to: number) => {
    const box = (await grip.boundingBox())!;
    const y = box.y + box.height / 2;
    const x = box.x + box.width / 2;
    await page.mouse.move(x, y);
    await page.mouse.down();
    await page.mouse.move(to, y, { steps: 6 });
    await page.mouse.up();
  };
  expect(await width()).toBe(220);
  // Wider, but never past the maximum.
  await drag(280);
  await expect.poll(width).toBeGreaterThan(260);
  await drag(900);
  await expect.poll(width).toBe(320);
  await drag(200);
  await expect.poll(width).toBeLessThanOrEqual(220);
  expect(await width()).toBeGreaterThanOrEqual(180);
  // Dragged below the fold: icons only, and the icons still navigate.
  await drag(40);
  await expect(side).toHaveAttribute('data-rail', 'true');
  await expect.poll(width).toBe(84);
  await page.getByTestId('nav-settings').click();
  await expect(page).toHaveURL(/#\/settings$/);
  await expect(page.getByTestId('nav-settings')).toHaveAttribute('aria-current', 'page');
  await page.reload();
  await expect(side).toHaveAttribute('data-rail', 'true');
  expect(await width()).toBe(84);
  // Double click on the edge brings the full sidebar back, and it stays open after a reload.
  await grip.dblclick();
  await expect(side).toHaveAttribute('data-rail', 'false');
  await expect.poll(width).toBeGreaterThanOrEqual(180);
  const open = await width();
  await page.reload();
  await expect(side).toHaveAttribute('data-rail', 'false');
  expect(await width()).toBe(open);
});
