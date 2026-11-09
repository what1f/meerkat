import { expect, type Page } from '@playwright/test';
import { test, rig } from './rig';
import { addFeishuMember, createFeishuTeam } from './feishu';

// The smallest window the app ships in.
test.use({ viewport: { width: 960, height: 660 } });

const wall = { rules: { x: 7, y: 1 }, tasks: { x: 24, y: 1 }, shift: { x: 34, y: 1 } };
// Hangs a saved wall once; later loads keep whatever the page has saved since.
const hang = (page: Page, workspace: string, layout: object = wall) =>
  page.addInitScript(
    ([id, saved]) => {
      if (!localStorage.getItem(`meerkat.team.wall.${id}`))
        localStorage.setItem(`meerkat.team.wall.${id}`, saved!);
    },
    [workspace, JSON.stringify(layout)],
  );
const open = async (page: Page, workspace: string, hash: string) => {
  await page.addInitScript((id) => {
    localStorage.setItem('meerkat.workspace', JSON.stringify(id));
    localStorage.setItem('meerkat.language', JSON.stringify('en'));
  }, workspace);
  await page.goto(`/#/${hash}`);
};
const noSideways = (page: Page) =>
  page.locator('main').evaluate((main) => main.scrollWidth <= main.clientWidth);
const inside = (page: Page, testId: string) =>
  page.locator('main').evaluate((main, id) => {
    const box = document.querySelector(`[data-testid="${id}"]`)!.getBoundingClientRect();
    const edge = main.getBoundingClientRect();
    return box.left >= edge.left && box.right <= edge.right;
  }, testId);

test('the member wall wraps to the columns that fit', async ({ page }) => {
  const workspace = 'min_wall';
  await rig({ op: 'create_workspace', workspace });
  const agents = ['ann', 'bob', 'cy', 'di'];
  for (const agent of agents) await rig({ op: 'create_agent', workspace, agent });
  await open(page, workspace, 'members');
  for (const agent of agents) await expect(page.getByTestId(`member-card-${agent}`)).toBeVisible();
  expect(await noSideways(page)).toBe(true);
  for (const agent of agents) expect(await inside(page, `member-card-${agent}`)).toBe(true);
});

test('the member page keeps the console and the badge in the window', async ({ page }) => {
  const workspace = 'min_detail';
  await rig({ op: 'create_workspace', workspace });
  await rig({ op: 'create_agent', workspace, agent: 'eve' });
  await open(page, workspace, 'member/eve');
  await expect(page.getByTestId('brain-picker')).toBeVisible();
  expect(await noSideways(page)).toBe(true);
  const badge = page.locator('main article').first();
  await badge.scrollIntoViewIfNeeded();
  const box = (await badge.boundingBox())!;
  expect(box.x).toBeGreaterThanOrEqual(0);
  expect(box.x + box.width).toBeLessThanOrEqual(960);
});

test('a team sign hung far to the right is brought back inside a narrow board', async ({
  page,
}) => {
  const workspace = 'min_team';
  await rig({ op: 'create_workspace', workspace, provider: 'feishu' });
  await page.addInitScript(
    ([id, wall]) => localStorage.setItem(`meerkat.team.wall.${id}`, wall!),
    [
      workspace,
      JSON.stringify({ rules: { x: 7, y: 1 }, tasks: { x: 24, y: 1 }, shift: { x: 34, y: 1 } }),
    ],
  );
  await open(page, workspace, 'team');
  await expect(page.getByTestId('team-shift')).toBeVisible();
  expect(await noSideways(page)).toBe(true);
  expect(await inside(page, 'team-shift')).toBe(true);
  expect(await inside(page, 'team-rules')).toBe(true);
});

test('the model picker stays in the window, shows levels and tells twin names apart', async ({
  page,
}) => {
  const workspace = 'min_picker';
  await rig({ op: 'create_workspace', workspace });
  await rig({ op: 'create_agent', workspace, agent: 'fay' });
  await open(page, workspace, 'member/fay');
  await page.getByTestId('brain-picker').click();
  const popup = page
    .getByRole('searchbox')
    .locator('xpath=ancestor::*[contains(@class,"popup")][1]');
  await expect(popup).toBeVisible();
  const box = (await popup.boundingBox())!;
  expect(box.y).toBeGreaterThanOrEqual(0);
  expect(box.y + box.height).toBeLessThanOrEqual(660);
  expect(box.x + box.width).toBeLessThanOrEqual(960);
  expect(
    await page.evaluate(
      () => document.scrollingElement!.scrollHeight <= document.scrollingElement!.clientHeight,
    ),
  ).toBe(true);
  // The scripted Harness reports two models called Twin: they carry their source, others stay bare.
  const options = page.getByRole('option');
  await expect(options.filter({ hasText: 'Twin Flash' })).toHaveCount(2);
  await expect(options.filter({ hasText: /Name\s*north$/ })).toHaveCount(1);
  await expect(options.filter({ hasText: /Name\s*south$/ })).toHaveCount(1);
  await expect(options.filter({ hasText: /^Scripted$/ })).toHaveCount(1);
  // A long name never spills out of its row: the hint is cut short first.
  for (const option of await options.all())
    expect(await option.evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
  const twin = options.filter({ hasText: /Name\s*north$/ });
  expect(await twin.locator('[data-hint]').evaluate((el) => el.scrollWidth > el.clientWidth)).toBe(
    true,
  );
  await expect(twin.locator('[data-label]')).toHaveCSS('font-family', /-apple-system/);
  await expect(twin.locator('[data-hint]')).toHaveCSS('font-family', /ui-monospace/);
  // The chosen model brings its own thinking levels.
  await options.filter({ hasText: /Name\s*north$/ }).click();
  const lever = page.getByRole('slider');
  await expect(lever).toHaveAttribute('aria-valuemax', '3');
  await options.filter({ hasText: /Name\s*south$/ }).click();
  await expect(lever).toHaveAttribute('aria-valuemax', '0');
});

test('off duty every screen is off and no member looks broken', async ({ page }) => {
  const workspace = 'min_off_duty';
  await rig({ op: 'create_workspace', workspace });
  await rig({ op: 'create_agent', workspace, agent: 'gus' });
  await rig({ op: 'set_duty', workspace, on: false });
  await open(page, workspace, 'members');
  await expect(page.getByTestId('member-card-gus')).toBeVisible();
  await expect(page.getByTestId('member-card-gus').locator('[data-status]')).not.toHaveAttribute(
    'data-status',
    'err',
  );
  await page.goto('/#/office');
  const screens = page.locator('[data-side]');
  await expect(screens.first()).toBeAttached();
  for (const screen of await screens.all())
    await expect(screen).toHaveAttribute('data-state', 'dark');
});

test('a human member without a picture wears a built-in avatar', async ({ page, baseURL }) => {
  const workspace = 'min_human';
  await rig({ op: 'create_workspace', workspace, provider: 'feishu', member_avatar: false });
  await rig({ op: 'feishu_scan', workspace, state: 'approved' });
  await addFeishuMember(baseURL!, workspace, 'hal');
  await open(page, workspace, 'members');
  const avatar = page.getByTestId(/^human-card-/).locator('article img:not([data-brand])');
  await expect(avatar).toHaveAttribute('src', /assets\/\d\d-.*\.jpg$/);
  await expect
    .poll(() => avatar.evaluate((img: HTMLImageElement) => img.complete && img.naturalWidth > 0))
    .toBe(true);
});

// The whole hung pieces: string, peg and paper.
const pieces = ['rules', 'tasks', 'shift'];
const boxes = async (page: Page) =>
  Promise.all(pieces.map(async (id) => (await page.locator(`[data-spot="${id}"]`).boundingBox())!));

test('moving one piece on a narrow board leaves the others as they were hung', async ({
  page,
  baseURL,
}) => {
  const workspace = 'min_keep';
  await createFeishuTeam(workspace);
  await addFeishuMember(baseURL!, workspace);
  await hang(page, workspace);
  await page.setViewportSize({ width: 1600, height: 900 });
  await open(page, workspace, 'team');
  await expect(page.getByTestId('team-shift')).toBeVisible();
  const [rules, tasks] = await boxes(page);
  const saved = () =>
    page.evaluate((id) => JSON.parse(localStorage.getItem(`meerkat.team.wall.${id}`)!), workspace);

  await page.setViewportSize({ width: 960, height: 660 });
  const sign = page.getByTestId('team-shift');
  await expect(sign).toBeVisible();
  await expect(page.locator('[data-settling]')).toHaveCount(0);
  // Let the wall settle into the narrow board before taking hold of anything.
  let last = '';
  await expect
    .poll(async () => {
      const now = JSON.stringify(await boxes(page));
      const still = now === last;
      last = now;
      return still;
    })
    .toBe(true);
  const at = (await sign.boundingBox())!;
  await page.mouse.move(at.x + at.width / 2, at.y + 4);
  await page.mouse.down();
  await page.mouse.move(at.x + at.width / 2 - 30, at.y + 240, { steps: 12 });
  await page.mouse.up();
  await expect.poll(async () => (await saved()).shift).not.toEqual(wall.shift);
  await expect(page.locator('[data-settling]')).toHaveCount(0);
  const after = await saved();
  expect(after.rules).toEqual(wall.rules);
  expect(after.tasks).toEqual(wall.tasks);
  expect(after.shift).not.toEqual(wall.shift);

  await page.setViewportSize({ width: 1600, height: 900 });
  await page.reload();
  await expect(page.getByTestId('team-shift')).toBeVisible();
  const [rulesBack, tasksBack] = await boxes(page);
  for (const [was, now] of [
    [rules, rulesBack],
    [tasks, tasksBack],
  ] as const) {
    expect(Math.abs(was!.x - now!.x)).toBeLessThan(2);
    expect(Math.abs(was!.y - now!.y)).toBeLessThan(2);
  }
});

const crowded = [
  { name: 'a wall hung wide on the smallest window', width: 960, layout: wall },
  {
    name: 'a rules sign hung low on the smallest window',
    width: 960,
    layout: { rules: { x: 12, y: 20 }, tasks: { x: 24, y: 1 }, shift: { x: 34, y: 1 } },
  },
  { name: 'a wall hung wide on a window just wider', width: 984, layout: wall },
];
for (const { name, width, layout } of crowded) {
  test(`${name} never lets pieces cover each other`, async ({ page, baseURL }) => {
    const workspace = `min_clash_${width}_${layout.rules.y}`;
    await createFeishuTeam(workspace);
    await addFeishuMember(baseURL!, workspace);
    await hang(page, workspace, layout);
    await page.setViewportSize({ width: 1600, height: 900 });
    await open(page, workspace, 'team');
    await page.getByTestId('tasklist').click();
    await expect(page.getByTestId('tasklist')).toHaveAttribute('aria-haspopup', 'dialog');

    await page.setViewportSize({ width, height: 660 });
    let last = '';
    await expect
      .poll(async () => {
        const now = JSON.stringify(await boxes(page));
        const still = now === last;
        last = now;
        return still;
      })
      .toBe(true);
    const [rules, tasks, shift] = await boxes(page);
    const apart = (a: typeof rules, b: typeof rules) =>
      a!.x + a!.width <= b!.x ||
      b!.x + b!.width <= a!.x ||
      a!.y + a!.height <= b!.y ||
      b!.y + b!.height <= a!.y;
    expect(apart(rules, tasks)).toBe(true);
    expect(apart(rules, shift)).toBe(true);
    expect(apart(tasks, shift)).toBe(true);
    expect(await noSideways(page)).toBe(true);
    await page.getByTestId('tasklist').click();
    await page.getByTestId('tasklist-board').getByRole('button', { name: 'More' }).click();
    await expect(page.getByRole('menuitem', { name: 'Turn off' })).toBeVisible();
    await page.keyboard.press('Escape');
    await page.keyboard.press('Escape');
    await expect(page.getByTestId('tasklist-board')).toBeHidden();
    await page.getByRole('switch').click();
    await expect(page.getByTestId('team-shift')).toHaveAttribute('data-on', 'false');
  });
}

const grown = [
  { name: 'a new team', layout: null },
  {
    name: 'a wall hung inside the board',
    layout: { rules: { x: 7, y: 1 }, tasks: { x: 7, y: 21 }, shift: { x: 25, y: 1 } },
  },
];
for (const { name, layout } of grown) {
  test(`${name} keeps its pieces apart when the rules grow taller`, async ({ page, baseURL }) => {
    const workspace = `min_grow_${layout ? 'saved' : 'new'}`;
    await createFeishuTeam(workspace);
    await addFeishuMember(baseURL!, workspace);
    if (layout) await hang(page, workspace, layout);
    await open(page, workspace, 'team');
    await page.getByTestId('tasklist').click();
    await expect(page.getByTestId('tasklist')).toHaveAttribute('aria-haspopup', 'dialog');

    const rulesBox = page.locator('[data-spot="rules"]');
    const short = (await rulesBox.boundingBox())!.height;
    await page
      .getByTestId('team-rules')
      .fill(Array.from({ length: 25 }, (_, i) => `Rule ${i + 1}`).join('\n'));
    await expect
      .poll(async () => (await rulesBox.boundingBox())!.height)
      .toBeGreaterThan(short + 300);
    let last = '';
    await expect
      .poll(async () => {
        const now = JSON.stringify(await boxes(page));
        const still = now === last;
        last = now;
        return still;
      })
      .toBe(true);
    const [rules, tasks, shift] = await boxes(page);
    const apart = (a: typeof rules, b: typeof rules) =>
      a!.x + a!.width <= b!.x ||
      b!.x + b!.width <= a!.x ||
      a!.y + a!.height <= b!.y ||
      b!.y + b!.height <= a!.y;
    expect(apart(rules, tasks)).toBe(true);
    expect(apart(rules, shift)).toBe(true);
    expect(apart(tasks, shift)).toBe(true);
    await page.getByTestId('tasklist').click();
    await page.getByTestId('tasklist-board').getByRole('button', { name: 'More' }).click();
    await expect(page.getByRole('menuitem', { name: 'Turn off' })).toBeVisible();
    await page.keyboard.press('Escape');
    await page.keyboard.press('Escape');
    await expect(page.getByTestId('tasklist-board')).toBeHidden();
    await page.getByRole('switch').click();
    await expect(page.getByTestId('team-shift')).toHaveAttribute('data-on', 'false');
  });
}
