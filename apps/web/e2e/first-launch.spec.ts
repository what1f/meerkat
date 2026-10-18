import { test as base, expect } from '@playwright/test';
import { startRig } from './rig-process';

// This test owns its Runtime: the data directory is new, with no team, whatever ran before.
const test = base.extend({
  baseURL: async ({}, use) => {
    const rig = await startRig();
    try {
      await use(rig.baseURL);
    } finally {
      await rig.cleanup();
    }
  },
});

test('A first launch with no team opens onboarding and stays on it', async ({ page }) => {
  await page.goto('/');
  const onboarding = page.getByTestId('onboarding');
  await expect(onboarding).toHaveAttribute('data-step', '0');
  // The first team appears while onboarding runs; the page must not be taken away.
  await page.getByTestId('onboarding-next').click();
  await expect(onboarding).toHaveAttribute('data-step', '1');
  await expect(page.getByTestId('onboarding-qr')).toBeVisible();
  await page.getByTestId('onboarding-later').click();
  await expect(onboarding).toHaveAttribute('data-step', '2');
  await page.getByTestId('onboarding-later').click();
  await expect(page).toHaveURL(/#\/members$/);
  await expect(onboarding).toHaveCount(0);
});
