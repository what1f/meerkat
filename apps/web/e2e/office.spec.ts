import { expect, type Browser, type Locator, type Page } from '@playwright/test';
import { test, rig } from './rig';

const member = { workspace: 'office_team', agent: 'bruce' };
let page: Page;
let paired = '';

async function api(op: string, body: object = {}) {
  const response = await page.request.post(`/api/v1/${op}`, { data: { request_id: op, ...body } });
  expect(response.ok(), await response.text()).toBe(true);
  return response.json();
}
const row = (id: string) => page.getByTestId(`machine-row-${id}`);
const seat = (within: Locator) => within.getByRole('button', { name: member.agent, exact: true });
async function carry(from: string, to: string) {
  const start = (await seat(row(from)).boundingBox())!;
  const target = (await row(to).boundingBox())!;
  await page.mouse.move(start.x + start.width / 2, start.y + start.height / 2);
  await page.mouse.down();
  await page.mouse.move(target.x + 120, target.y + target.height / 2, { steps: 12 });
  await page.mouse.up();
  await expect(seat(row(to))).toBeVisible();
  await expect(seat(row(from))).toHaveCount(0);
}
const localMachine = async () =>
  (await api('get_machine_catalog')).machines.find((m: { is_local: boolean }) => m.is_local)
    .machine_id as string;
async function openOffice(browser: Browser, baseURL?: string, clock = false) {
  const context = await browser.newContext({
    baseURL,
    permissions: ['clipboard-read', 'clipboard-write'],
  });
  await context.addInitScript((workspace) => {
    localStorage.setItem('lankide.workspace', JSON.stringify(workspace));
    localStorage.setItem('lankide.language', JSON.stringify('en'));
  }, member.workspace);
  const opened = await context.newPage();
  if (clock) await opened.clock.install();
  await opened.goto('/#/office');
  return opened;
}
async function openMenu(id: string, item: string) {
  await row(id).hover();
  await page.getByTestId(`machine-menu-${id}`).click();
  await page.getByRole('menuitem', { name: item }).click();
}

test.describe.serial('office', () => {
  test.beforeAll(async ({ browser, baseURL }) => {
    await rig({ op: 'create_workspace', workspace: member.workspace });
    await rig({ op: 'create_agent', ...member });
    page = await openOffice(browser, baseURL);
  });
  test.afterAll(async () => {
    await page.context().close();
  });

  test('the copied pairing command adds a new online office row', async () => {
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('Office');
    await page.getByTestId('add-machine').click();
    const command = page.getByTestId('pair-command');
    await expect(command).toContainText('lankide-machine pair wss://');
    await expect(page.getByTestId('pair-state')).toHaveAttribute('data-connected', 'false');
    // Point the command at the rig's loopback listener, the way a user fixes a wrong address.
    await page.getByRole('button', { name: 'Network' }).click();
    const address = page.getByTestId('remote-address');
    const port = new URL(`wss://${await address.inputValue()}`).port;
    await address.fill(`127.0.0.1:${port}`);
    await address.press('Enter');
    await expect(command).toContainText(`wss://127.0.0.1:${port} `);
    await command.getByRole('button', { name: 'Copy' }).click();
    const copied = await page.evaluate(() => navigator.clipboard.readText());
    expect(copied).toBe(await command.textContent());
    paired = (await rig({ op: 'pair_machine', command: copied })).machine_id!;
    const name = (await api('get_machine_catalog')).machines.find(
      (m: { machine_id: string }) => m.machine_id === paired,
    ).display_name;
    await expect(page.getByTestId('pair-state')).toHaveAttribute('data-connected', 'true');
    await expect(page.getByTestId('pair-state')).toHaveText(name);
    await expect(row(paired)).toHaveAttribute('data-status', 'ok');
    await expect(row(paired)).toContainText(name);
    await page.getByRole('button', { name: 'Done' }).click();
    await expect(command).toBeHidden();
  });

  test('a member carried to the new row runs there next time', async () => {
    const local = await localMachine();
    await expect(seat(row(local))).toBeVisible();
    await carry(local, paired);
    await rig({ op: 'hold_next_run', ...member });
    await rig({ op: 'mention', ...member, chat: 'group', person: 'alice', text: 'go' });
    expect((await rig({ op: 'wait_running', ...member })).machine_id).toBe(paired);
    await rig({ op: 'release_run', ...member });
    await rig({ op: 'settle' });
    await page.reload();
    await expect(seat(row(paired))).toBeVisible();
    await seat(row(paired)).click();
    await expect(page).toHaveURL(new RegExp(`#/member/${member.agent}$`));
    await page.goBack();
  });

  test('renaming in place keeps the new name after a reload', async () => {
    await openMenu(paired, 'Rename');
    const editor = row(paired).getByRole('textbox');
    // The editor must survive the menu handing focus back as it closes.
    await expect(page.getByRole('menu')).toHaveCount(0);
    await expect(editor).toBeFocused();
    await editor.fill('Studio');
    await editor.press('Enter');
    await expect(editor).toBeHidden();
    await expect(row(paired).locator('header')).toHaveText('Studio');
    await page.reload();
    await expect(row(paired).locator('header')).toHaveText('Studio');
    // Escape leaves the name as it was.
    await openMenu(paired, 'Rename');
    await row(paired).getByRole('textbox').fill('Discarded');
    await row(paired).getByRole('textbox').press('Escape');
    await expect(row(paired).locator('header')).toHaveText('Studio');
  });

  test('removing asks for confirmation and takes the row away', async () => {
    // A computer with members on it stays until they move.
    await openMenu(paired, 'Remove');
    await expect(page.getByText('Move the members here to another computer first')).toBeVisible();
    await expect(page.getByTestId('confirm-remove-machine')).toBeHidden();
    await carry(paired, await localMachine());
    await openMenu(paired, 'Remove');
    const dialog = page.getByTestId('confirm-remove-machine');
    await expect(dialog).toContainText('Remove Studio?');
    await dialog.getByRole('button', { name: 'Cancel' }).click();
    await expect(dialog).toBeHidden();
    await expect(row(paired)).toBeVisible();
    await openMenu(paired, 'Remove');
    await dialog.getByRole('button', { name: 'Remove' }).click();
    await expect(row(paired)).toHaveCount(0);
    const machine = (await api('get_machine_catalog')).machines.find(
      (m: { machine_id: string }) => m.machine_id === paired,
    );
    expect(machine.revoked_at).not.toBeNull();
    await page.reload();
    await expect(row(paired)).toHaveCount(0);
  });

  test('a pairing command left open renews its code and still pairs', async ({
    browser,
    baseURL,
  }) => {
    const later = await openOffice(browser, baseURL, true);
    await later.getByTestId('add-machine').click();
    const command = later.getByTestId('pair-command');
    await expect(command).toContainText('lankide-machine pair wss://127.0.0.1:');
    const first = (await command.textContent())!;
    await later.clock.fastForward('10:00');
    await expect(command).not.toHaveText(first);
    const renewed = (await command.textContent())!;
    const machine = (await rig({ op: 'pair_machine', command: renewed })).machine_id!;
    await expect(later.getByTestId('pair-state')).toHaveAttribute('data-connected', 'true');
    await expect(later.getByTestId(`machine-row-${machine}`)).toBeVisible();
    await api('revoke_machine', { machine_id: machine });
    await later.context().close();
  });
});
