import { readFile } from 'node:fs/promises';
import { expect } from '@playwright/test';
import { test, rig } from './rig';

// Only the User-Agent changes. The UI still talks to the real Runtime.
test('Desktop reserves an unobstructed title strip and keeps controls clickable', async ({
  browser,
  baseURL,
}) => {
  await rig({ op: 'create_workspace', workspace: 'desktop_shell' });
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
        return document.elementFromPoint(x + width / 2, y + 20) === element;
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
        await expect(page.locator('.peek img')).toHaveCSS('top', '22px');
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
