import { readFile } from 'node:fs/promises';
import { expect, type Browser, type Page } from '@playwright/test';
import { test, rig } from './rig';

// The Runtime and all business data are real; only the host User-Agent differs.
for (const desktop of [false, true]) {
  test(`${desktop ? 'Desktop' : 'Browser'}: dock navigation, title clearance and last-row access`, async ({
    browser,
    baseURL,
  }) => {
    const workspace = desktop ? 'desktop_dock' : 'browser_dock';
    await rig({ op: 'create_workspace', workspace });
    for (let i = 0; i < 11; i++) {
      await rig({ op: 'create_agent', workspace, agent: `desk${i}` });
    }
    const context = await browser.newContext({
      baseURL,
      reducedMotion: 'reduce',
      ...(desktop ? { userAgent: 'Mozilla/5.0 LankideDesktop/0.1.0' } : {}),
    });
    const page = await context.newPage();
    try {
      await page.addInitScript(
        (id) => localStorage.setItem('lankide.workspace', JSON.stringify(id)),
        workspace,
      );
      for (const viewport of [
        { width: 1240, height: 820 },
        { width: 960, height: 660 },
      ]) {
        await page.setViewportSize(viewport);
        await page.goto('/#/settings');
        const bar = page.locator('header').filter({ has: page.locator('h1') });
        await expect(bar.locator('h1')).toBeVisible();
        expect((await bar.boundingBox())!.height).toBe(48);
        expect((await bar.locator('h1').boundingBox())!.x).toBe(desktop ? 92 : 24);
        const centeredDock = async () => {
          const dock = (await page.getByTestId('dock').boundingBox())!;
          const size = await page.evaluate(() => ({ width: innerWidth, height: innerHeight }));
          expect(dock.x + dock.width / 2).toBeCloseTo(size.width / 2, 1);
          expect(size.height - dock.y - dock.height).toBe(18);
          expect(dock.height).toBe(44);
        };
        for (const theme of ['light', 'dark']) {
          await page.getByTestId(`settings-theme-${theme}`).click();
          await centeredDock();
        }
        // Real browser fullscreen enters the same layout as the host-published native state.
        await page.evaluate(() => document.documentElement.requestFullscreen());
        await expect.poll(() => page.evaluate(() => !!document.fullscreenElement)).toBe(true);
        await expect.poll(async () => (await bar.locator('h1').boundingBox())!.x).toBe(24);
        await centeredDock();
        await page.evaluate(() => document.exitFullscreen());
        await expect
          .poll(async () => (await bar.locator('h1').boundingBox())!.x)
          .toBe(desktop ? 92 : 24);
        for (const name of ['team', 'office', 'members']) {
          await page.getByTestId(`nav-${name}`).click();
          await expect(page).toHaveURL(new RegExp(`#/${name}$`));
          await expect(page.getByTestId(`nav-${name}`)).toHaveAttribute('aria-current', 'page');
          await centeredDock();
        }
        const last = page.getByTestId('member-card-desk10');
        await expect(last).toBeVisible();
        await page.locator('main').evaluate((main) => {
          main.scrollTop = main.scrollHeight;
        });
        const dock = (await page.getByTestId('dock').boundingBox())!;
        const card = (await last.locator('article').boundingBox())!;
        expect(card.y + card.height).toBeLessThan(dock.y);
        await last.click();
        await expect(page.getByTestId('member-name')).toBeVisible();
        // The member is held up over the wall; Esc puts them back.
        await expect(page.getByTestId('sheet')).toBeVisible();
        await page.keyboard.press('Escape');
        await expect(page).toHaveURL(/#\/members$/);
        await expect(page.getByTestId('member-name')).toBeHidden();
        if (desktop) {
          const drag = await page.locator('header[data-tauri-drag-region]').evaluate((header) => {
            const r = header.getBoundingClientRect();
            return document.elementFromPoint(r.right - 80, r.y + r.height / 2) === header;
          });
          expect(drag).toBe(true);
        }
        await page.getByTestId('team-switcher').click();
        const menu = (await page.getByRole('menu').boundingBox())!;
        const trigger = (await page.getByTestId('team-switcher').boundingBox())!;
        expect(menu.y + menu.height).toBeLessThanOrEqual(trigger.y);
        await page.getByTestId('team-settings').click();
        await expect(page).toHaveURL(/#\/settings$/);
        await expect(page.getByTestId('settings-language')).toBeVisible();
      }
    } finally {
      await context.close();
    }
  });
}

for (const locale of ['en-US', 'zh-CN']) {
  for (const colorScheme of ['light', 'dark'] as const) {
    test(`Boot content is visible without an animation tick: ${locale}, ${colorScheme}`, async ({
      browser,
    }) => {
      const context = await browser.newContext({ locale, colorScheme });
      const page = await context.newPage();
      try {
        // Install before parsing so even a slow image load cannot advance CSS animations.
        await page.addInitScript(() => {
          const pauseAnimations = () => {
            if (!document.head) return;
            const style = document.createElement('style');
            style.textContent = '* { animation-play-state: paused !important; }';
            document.head.append(style);
            observer.disconnect();
          };
          const observer = new MutationObserver(pauseAnimations);
          observer.observe(document, { childList: true, subtree: true });
          pauseAnimations();
        });
        // Network boundary: serve the shipped boot assets, without changing their contents.
        await page.route('http://boot.test/**', async (route) => {
          const name = new URL(route.request().url()).pathname.slice(1);
          if (name.endsWith('.png')) await new Promise((resolve) => setTimeout(resolve, 1500));
          await route.fulfill({
            body: await readFile(new URL(`../../desktop/boot/${name}`, import.meta.url)),
            contentType: name.endsWith('.png') ? 'image/png' : 'text/html',
          });
        });
        await page.goto('http://boot.test/boot.html');
        await expect(page.locator('#loading')).toHaveText(
          locale === 'zh-CN' ? '正在打开 Lankide…' : 'Opening Lankide…',
        );
        await expect(page.locator('#loading')).toHaveCSS('opacity', '1');
        await expect(page.locator('.peek .meerkat')).toBeAttached();
        await expect(page.locator('.peek .eye')).toHaveCount(2);
        expect(
          await page
            .locator('.peek img')
            .evaluate((image: HTMLImageElement) => image.complete && image.naturalWidth > 0),
        ).toBe(true);
        await page.goto('http://boot.test/boot.html#startup-failed');
        await expect(page.getByRole('button')).toHaveText(locale === 'zh-CN' ? '重试' : 'Retry');
        await expect(page.locator('#loader')).toBeHidden();
        await expect(page.locator('#error')).toBeVisible();
      } finally {
        await context.close();
      }
    });
  }
}

const bootPeek = async (browser: Browser, reducedMotion: 'reduce' | 'no-preference') => {
  const context = await browser.newContext({ reducedMotion });
  const page = await context.newPage();
  // Network boundary: serve the shipped boot assets, without changing their contents.
  await page.route('http://boot.test/**', async (route) => {
    const name = new URL(route.request().url()).pathname.slice(1);
    await route.fulfill({
      body: await readFile(new URL(`../../desktop/boot/${name}`, import.meta.url)),
      contentType: name.endsWith('.png') ? 'image/png' : 'text/html',
    });
  });
  await page.goto('http://boot.test/boot.html');
  return { context, page };
};
const peekState = (page: Page) =>
  page.evaluate(() => ({
    top: document.querySelector<HTMLElement>('.meerkat')!.style.top,
    eyes: [...document.querySelectorAll<HTMLElement>('.eye')].map(
      (eye) => `${eye.style.left}/${eye.style.top}/${eye.style.height}`,
    ),
  }));

test('Boot meerkat climbs out of its burrow and its eyes move', async ({ browser }) => {
  const { context, page } = await bootPeek(browser, 'no-preference');
  try {
    await expect(page.locator('.peek .eye')).toHaveCount(2);
    const start = await peekState(page);
    await expect.poll(async () => (await peekState(page)).top).not.toBe(start.top);
    await expect.poll(async () => (await peekState(page)).eyes.join()).not.toBe(start.eyes.join());
  } finally {
    await context.close();
  }
});

test('Boot meerkat only peeks, without moving, when motion is reduced', async ({ browser }) => {
  const { context, page } = await bootPeek(browser, 'reduce');
  try {
    await expect(page.locator('.peek .eye')).toHaveCount(2);
    const still = await peekState(page);
    expect(still.top).toBe('94px');
    await page.waitForTimeout(1200);
    expect(await peekState(page)).toEqual(still);
  } finally {
    await context.close();
  }
});

for (const viewport of [
  { width: 1240, height: 820 },
  { width: 960, height: 660 },
]) {
  test(`Desktop ${viewport.width}px: putting a paused member away, the resume notice and the drag bar all work`, async ({
    browser,
    baseURL,
  }) => {
    const member = { workspace: `desktop_notice_${viewport.width}`, agent: 'linus' };
    await rig({ op: 'create_workspace', workspace: member.workspace });
    await rig({ op: 'create_agent', ...member });
    const context = await browser.newContext({
      baseURL,
      userAgent: 'Mozilla/5.0 LankideDesktop/0.1.0',
      viewport,
    });
    const page = await context.newPage();
    try {
      await page.addInitScript((id) => {
        localStorage.setItem('lankide.workspace', JSON.stringify(id));
      }, member.workspace);
      await page.goto(`/#/member/${member.agent}`);
      await page.getByRole('button', { name: /高级|Advanced/ }).click();
      const names = page.getByRole('textbox', { name: /^(名称|Variable)$/ });
      const values = page.getByRole('textbox', { name: /^(值|Value)$/ });
      const blank = (await names.count()) - 1;
      await names.nth(blank).fill('LANKIDE_SYSTEM_SCRIPT');
      await values.nth(blank).fill('{"behaviour":"fail_spawn","text":"model not found"}');
      await page.getByTestId('member-save').click();
      await expect(page.getByTestId('member-save')).toBeDisabled();
      await rig({ op: 'mention', ...member, chat: 'group', person: 'alice', text: 'hello' });
      await expect(page.getByTestId('member-pause')).toBeEnabled();
      await page.goto('/#/members');
      await page.getByTestId(`member-card-${member.agent}`).click();
      const resume = page.getByTestId('sheet').locator('[data-tone] button');
      await expect(resume).toBeVisible();
      // The empty bar is still the drag handle.
      const drag = await page.locator('header[data-tauri-drag-region]').evaluate((header) => {
        const { x, y, width, height } = header.getBoundingClientRect();
        const hit = document.elementFromPoint(x + width - 8, y + height / 2);
        return hit === header;
      });
      expect(drag).toBe(true);
      // While the notice is up and the member is paused, empty space beside them still puts them away.
      await expect(page.getByTestId('member-pause')).toBeEnabled();
      await page.getByTestId('sheet').click({ position: { x: 12, y: viewport.height / 2 } });
      await expect(page).toHaveURL(/#\/members$/);
      // Opened again, the notice reaches into the title strip; its upper edge is still its own.
      await page.getByTestId(`member-card-${member.agent}`).click();
      await expect(resume).toBeVisible();
      await resume.click({ position: { x: 10, y: 2 } });
      await expect(page.getByTestId('member-pause')).toBeDisabled();
    } finally {
      await context.close();
    }
  });
}
