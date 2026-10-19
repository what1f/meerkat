import { expect } from '@playwright/test';
import { test, rig } from './rig';

test('language and appearance chosen in settings survive a reload', async ({ page }) => {
  await rig({ op: 'create_workspace', workspace: 'settings_team' });
  await page.emulateMedia({ colorScheme: 'light' });
  await page.goto('/#/members');
  await page.getByTestId('team-switcher').click();
  await page.getByTestId('team-settings').click();
  const html = page.locator('html');
  await expect(html).toHaveAttribute('data-theme', 'light');
  await expect(page.getByTestId('settings-theme-system')).toHaveAttribute('aria-pressed', 'true');

  await page.getByTestId('settings-language').click();
  await page.getByRole('option', { name: '简体中文' }).click();
  await expect(page.locator('header h1')).toHaveText('设置');
  await expect(html).toHaveAttribute('lang', 'zh');

  await page.getByTestId('settings-theme-dark').click();
  await expect(html).toHaveAttribute('data-theme', 'dark');
  await expect(page.getByTestId('settings-theme-dark')).toHaveAttribute('aria-pressed', 'true');

  await page.reload();
  await expect(page.locator('header h1')).toHaveText('设置');
  await expect(page.getByTestId('settings-language')).toContainText('简体中文');
  await expect(html).toHaveAttribute('data-theme', 'dark');
  await expect(page.getByTestId('settings-theme-dark')).toHaveAttribute('aria-pressed', 'true');

  // Following the system again tracks the system appearance after a reload too.
  await page.getByTestId('settings-theme-system').click();
  await expect(html).toHaveAttribute('data-theme', 'light');
  await page.emulateMedia({ colorScheme: 'dark' });
  await expect(html).toHaveAttribute('data-theme', 'dark');
  await page.reload();
  await expect(page.getByTestId('settings-theme-system')).toHaveAttribute('aria-pressed', 'true');
  await expect(html).toHaveAttribute('data-theme', 'dark');

  await page.getByTestId('settings-language').click();
  await page.getByRole('option', { name: 'English' }).click();
  await page.reload();
  await page.getByTestId('team-switcher').click();
  await expect(page.getByTestId('team-settings')).toHaveText('Settings');
  await page.getByTestId('team-settings').click();
});
