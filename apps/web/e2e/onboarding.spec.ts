import { expect, request as playwrightRequest, type Page } from '@playwright/test';
import { test, rig } from './rig';

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
type Listed = { workspaces: { workspace: { workspace_id: string }; provider_id: string }[] };
const workspaces = async (baseURL: string) =>
  ((await api(baseURL, 'list_workspaces')) as Listed).workspaces.map((w) => ({
    id: w.workspace.workspace_id,
    provider: w.provider_id,
  }));
// The team onboarding created: the only new one for that chat app.
async function created(baseURL: string, before: string[], provider: string) {
  let id = '';
  await expect
    .poll(async () => {
      const found = (await workspaces(baseURL)).filter(
        (w) => w.provider === provider && !before.includes(w.id),
      );
      id = found[0]?.id ?? '';
      return found.length;
    })
    .toBe(1);
  return id;
}
async function onlyAgent(baseURL: string, workspace: string) {
  const listed = await api(baseURL, 'list_agents', { workspace: { workspace_id: workspace } });
  const agents = listed.agents as { agent: { agent_id: string }; name: string }[];
  expect(agents).toHaveLength(1);
  return agents[0]!.agent.agent_id;
}
const step = (page: Page, n: number) =>
  expect(page.getByTestId('onboarding')).toHaveAttribute('data-step', String(n));
const next = (page: Page) => page.getByTestId('onboarding-next');

async function begin(page: Page, baseURL: string) {
  const before = (await workspaces(baseURL)).map((w) => w.id);
  await page.goto('/#/onboarding');
  await step(page, 0);
  await next(page).click();
  await step(page, 1);
  return before;
}
// Choose the scripted AI once its test settles, then name the first member.
async function chooseAiAndCreate(page: Page) {
  await step(page, 2);
  // The web rig's scripted AI cannot answer a test yet; either result settles the row.
  await expect(page.getByTestId('harness-pi')).toHaveAttribute('data-test', /^(ok|fail)$/);
  await next(page).click();
  await step(page, 3);
  await expect(next(page)).toBeDisabled();
  await page.getByTestId('onboarding-member-name').fill('Alpha');
  await next(page).click();
}
// The team lead says hello in the chat app; onboarding ends on the member's first reply.
async function firstReply(page: Page, baseURL: string, workspace: string) {
  await step(page, 4);
  const chat = page.getByTestId('onboarding-chat');
  await expect(chat).not.toContainText('Alpha');
  await expect(next(page)).not.toHaveAttribute('data-variant', 'primary');
  const agent = await onlyAgent(baseURL, workspace);
  await rig({ op: 'dm', workspace, agent, text: 'hello' });
  await expect(chat).toContainText('Alpha');
  await expect(next(page)).toHaveAttribute('data-variant', 'primary');
  await next(page).click();
  await expect(page).toHaveURL(/#\/members$/);
  // The members board opens on the team onboarding just made.
  expect(await page.evaluate(() => localStorage.getItem('lankide.workspace'))).toBe(
    JSON.stringify(workspace),
  );
  // The team switcher shows the new team, and its board holds the new member.
  await expect(page.getByTestId('team-switcher')).toHaveText(/^(My team|我的团队)$/);
  await expect(page.getByText('Alpha')).toBeVisible();
}

test('Feishu: scan through every code state, then the first member answers the team lead', async ({
  page,
  baseURL,
}) => {
  test.setTimeout(90_000);
  const before = await begin(page, baseURL!);
  const workspace = await created(baseURL!, before, 'feishu');
  const qr = page.getByTestId('onboarding-qr');
  await rig({ op: 'feishu_scan', workspace, state: 'waiting' });
  await expect(qr).toHaveAttribute('data-state', 'wait');
  await expect(next(page)).toBeDisabled();
  const first = await qr.locator('svg').innerHTML();
  await rig({ op: 'feishu_scan', workspace, state: 'scanned' });
  await expect(qr).toHaveAttribute('data-state', 'scanned');
  // An expired code is replaced by a new one to scan.
  await rig({ op: 'feishu_scan', workspace, state: 'expired' });
  await expect(qr).toHaveAttribute('data-state', 'wait');
  await expect.poll(() => qr.locator('svg').innerHTML()).not.toBe(first);
  await rig({ op: 'feishu_scan', workspace, state: 'complete' });
  await expect(next(page)).toBeEnabled();
  await expect(qr).toHaveCount(0);
  await next(page).click();
  await chooseAiAndCreate(page);
  await firstReply(page, baseURL!, workspace);
});

test('Telegram: a bad token is refused, the manager token connects and the member answers', async ({
  page,
  baseURL,
}) => {
  test.setTimeout(90_000);
  const before = await begin(page, baseURL!);
  await page.getByTestId('onboarding-provider').getByRole('radio').nth(1).click();
  const token = page.getByTestId('onboarding-token').getByRole('textbox');
  await expect(next(page)).toBeDisabled();
  await token.fill('100:wrong');
  await next(page).click();
  await expect(token).toHaveAttribute('aria-invalid', 'true');
  await step(page, 1);
  await token.fill('100:manager');
  await expect(token).not.toHaveAttribute('aria-invalid', 'true');
  await next(page).click();
  const workspace = await created(baseURL!, before, 'telegram');
  await chooseAiAndCreate(page);
  await firstReply(page, baseURL!, workspace);
});

test('Skipping the connection also skips the first member', async ({ page, baseURL }) => {
  test.setTimeout(60_000);
  // The AI on this Mac cannot be read at first; a retry brings it back.
  let offline = true;
  await page.route('**/api/v1/get_machine_catalog', (route) =>
    offline ? route.abort() : route.continue(),
  );
  await begin(page, baseURL!);
  await page.getByTestId('onboarding-later').click();
  await step(page, 2);
  const retry = page.getByTestId('onboarding-retry');
  await expect(retry).toBeVisible({ timeout: 20_000 });
  await expect(next(page)).toBeDisabled();
  offline = false;
  await retry.click();
  await expect(retry).toHaveCount(0);
  // The web rig's scripted AI cannot answer a test yet; either result settles the row.
  await expect(page.getByTestId('harness-pi')).toHaveAttribute('data-test', /^(ok|fail)$/);
  await next(page).click();
  await expect(page).toHaveURL(/#\/members$/);
});

async function telegramToken(page: Page) {
  await page.getByTestId('onboarding-provider').getByRole('radio').nth(1).click();
  await page.getByTestId('onboarding-token').getByRole('textbox').fill('100:manager');
}

test('A late Telegram answer does not move the Feishu flow on', async ({ page, baseURL }) => {
  test.setTimeout(90_000);
  let release!: () => void;
  const held = new Promise<void>((resolve) => (release = resolve));
  let arrived!: () => void;
  const answered = new Promise<void>((resolve) => (arrived = resolve));
  await page.route('**/api/v1/create_workspace', async (route) => {
    if (route.request().postDataJSON().provider_id !== 'telegram') return route.continue();
    const response = await route.fetch();
    arrived();
    await held;
    await route.fulfill({ response });
  });
  const before = await begin(page, baseURL!);
  await telegramToken(page);
  await next(page).click();
  await answered;
  await page.getByTestId('onboarding-provider').getByRole('radio').nth(0).click();
  release();
  const workspace = await created(baseURL!, before, 'telegram');
  await step(page, 1);
  // Back on Telegram, the team that was just made is used, and the member joins it.
  await page.getByTestId('onboarding-provider').getByRole('radio').nth(1).click();
  await next(page).click();
  await chooseAiAndCreate(page);
  await firstReply(page, baseURL!, workspace);
});

test('Skipping an AI that could not be read ends onboarding', async ({ page, baseURL }) => {
  test.setTimeout(60_000);
  await page.route('**/api/v1/get_machine_catalog', (route) => route.abort());
  await begin(page, baseURL!);
  await telegramToken(page);
  await next(page).click();
  await step(page, 2);
  await expect(page.getByTestId('onboarding-retry')).toBeVisible({ timeout: 25_000 });
  await page.getByTestId('onboarding-later').click();
  await expect(page).toHaveURL(/#\/members$/);
});

test('A first reply that could not be read can be retried', async ({ page, baseURL }) => {
  test.setTimeout(90_000);
  let offline = false;
  const waiting = page.waitForResponse('**/api/v1/get_agent_first_reply');
  await page.route('**/api/v1/get_agent_first_reply', (route) =>
    offline ? route.abort() : route.continue(),
  );
  const before = await begin(page, baseURL!);
  await telegramToken(page);
  await next(page).click();
  const workspace = await created(baseURL!, before, 'telegram');
  await chooseAiAndCreate(page);
  await step(page, 4);
  await waiting;
  offline = true;
  const agent = await onlyAgent(baseURL!, workspace);
  await rig({ op: 'dm', workspace, agent, text: 'hello' });
  const retry = page.getByTestId('onboarding-retry');
  await expect(retry).toBeVisible({ timeout: 30_000 });
  offline = false;
  await retry.click();
  await expect(page.getByTestId('onboarding-chat')).toContainText('Alpha');
  await expect(next(page)).toHaveAttribute('data-variant', 'primary');
});
