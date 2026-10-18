import { readFile } from 'node:fs/promises';
import { expect, type Browser, type Page } from '@playwright/test';
import { test, rig } from './rig';

// Only the User-Agent changes. The UI still talks to the real Runtime.
test('Desktop reserves an unobstructed title strip and keeps controls clickable', async ({
  browser,
  baseURL,
}) => {
  await rig({ op: 'create_workspace', workspace: 'desktop_shell' });
  await rig({ op: 'create_workspace', workspace: 'desktop_wall' });
  await rig({ op: 'create_agent', workspace: 'desktop_wall', agent: 'wall' });
  const context = await browser.newContext({
    baseURL,
    userAgent: 'Mozilla/5.0 LankideDesktop/0.1.0',
  });
  const page = await context.newPage();
  try {
    await page.goto('/#/settings');
    const strip = page.locator('[data-desktop-safe-area]');
    await expect(strip).toHaveAttribute('data-tauri-drag-region', 'true');
    expect(
      await strip.evaluate((element) => {
        const { x, y, width } = element.getBoundingClientRect();
        // Clear of the sidebar toggle beside the traffic lights.
        return document.elementFromPoint(x + width - 16, y + 20) === element;
      }),
    ).toBe(true);
    await page.getByTestId('nav-members').click();
    await expect(page).toHaveURL(/#\/members$/);
    await page.getByTestId('add-member').click();
    await page
      .locator('header')
      .getByRole('button', { name: /Members|成员/, exact: true })
      .click();
    await expect(page).toHaveURL(/#\/members$/);
    // The empty bar stays a drag handle on the wall and the open board, above no content.
    const bar = () =>
      page.locator('header[data-tauri-drag-region]').evaluate((header) => {
        const { x, y, width, height } = header.getBoundingClientRect();
        const hit = document.elementFromPoint(x + width - 80, y + height / 2);
        return { drag: hit?.closest('[data-tauri-drag-region]') === hit, bottom: y + height };
      });
    await page.addInitScript((id) => {
      localStorage.setItem('lankide.workspace', JSON.stringify(id));
    }, 'desktop_wall');
    await page.goto('/#/members');
    await page.reload();
    await expect(page.locator('div[data-desktop]')).toHaveAttribute('data-board', 'wall');
    await expect(page.locator('main [data-layout="rails"]')).toBeVisible();
    await expect(page.getByTestId('member-card-wall')).toBeVisible();
    expect((await bar()).drag).toBe(true);
    await page.getByTestId('add-member').click();
    await expect(page.getByTestId('member-name')).toBeVisible();
    await expect(page.locator('div[data-desktop]')).toHaveAttribute('data-board', 'open');
    expect((await bar()).drag).toBe(true);
    await page.goto('/#/team');
    const name = page.getByTestId('team-name');
    await expect(name).toBeVisible();
    const { drag, bottom } = await bar();
    expect(drag).toBe(true);
    expect((await name.boundingBox())!.y).toBeGreaterThanOrEqual(bottom);
    await page.goto('/#/onboarding');
    await expect(strip).toHaveAttribute('data-tauri-drag-region', 'true');
  } finally {
    await context.close();
  }
});

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
  test(`Desktop ${viewport.width}px: the back button, the resume notice and the drag bar of a paused member all work`, async ({
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
      const resume = page.locator('main [data-tone] button');
      await expect(resume).toBeVisible();
      // The empty bar is still the drag handle.
      const drag = await page.locator('header[data-tauri-drag-region]').evaluate((header) => {
        const { x, y, width, height } = header.getBoundingClientRect();
        const hit = document.elementFromPoint(x + width - 8, y + height / 2);
        return hit === header;
      });
      expect(drag).toBe(true);
      // While the notice is up and the member is paused, the back button keeps its lower edge.
      const back = page
        .locator('header')
        .getByRole('button', { name: /Members|成员/, exact: true });
      const box = (await back.boundingBox())!;
      await expect(page.getByTestId('member-pause')).toBeEnabled();
      await back.click({ position: { x: box.width / 2, y: box.height - 2 } });
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
