import { defineConfig, devices } from '@playwright/test';

export default defineConfig({
  testDir: './e2e',
  globalSetup: './e2e/global-setup.ts',
  fullyParallel: false,
  workers: 1,
  timeout: 45_000,
  expect: { timeout: 15_000 },
  outputDir: '../../target/e2e-artifacts/web/results',
  reporter: [['list']],
  use: { ...devices['Desktop Chrome'], headless: true, trace: 'retain-on-failure' },
});
