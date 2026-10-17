import { expect } from '@playwright/test';
import { test, rig } from './rig';

test('B4.1: navigate and receive a new workspace without a page reload', async ({ page }) => {
  const errors: string[] = [];
  page.on('pageerror', (error) => errors.push(error.message));
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
  await rig({ op: 'create_workspace', workspace: 'browser_team' });
  await frame;
  await expect(page.getByRole('combobox')).toContainText('browser_team');
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
