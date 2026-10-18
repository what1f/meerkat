import { expect } from '@playwright/test';
import { test, rig } from './rig';
import { api, bindOwner, signInFeishu } from './feishu';

for (const language of ['en', 'zh'] as const) {
  test(`owner re-login repairs an old App and exposes recoverable failures (${language})`, async ({
    page,
    baseURL,
  }, testInfo) => {
    const workspace = `owner_repair_${language}`;
    await signInFeishu(baseURL!, workspace);
    await bindOwner(baseURL!, workspace);
    const invoke = (action: string) =>
      api(baseURL!, 'invoke_provider_interface', {
        workspace: { workspace_id: workspace },
        interface_id: 'feishu.workspace.console_login',
        input_json: JSON.stringify({ action }),
      });
    await api(baseURL!, 'update_workspace', {
      workspace: { workspace_id: workspace },
      provider_config_json: JSON.stringify({ domain: 'feishu', task_system_enabled: true }),
      provider_secrets_json: null,
    });
    await rig({ op: 'feishu_authorization', workspace, revoked: true });
    const lapsed = await page.request.post('/api/v1/invoke_provider_interface', {
      data: {
        request_id: 'expire',
        workspace: { workspace_id: workspace },
        interface_id: 'feishu.workspace.tasks',
        input_json: '{"action":"list"}',
      },
    });
    expect((await lapsed.json()).detail).toBe('authorization_required');
    await rig({ op: 'feishu_app_configuration', workspace, denied: true });
    await rig({ op: 'feishu_scan', workspace, state: 'waiting' });
    await page.addInitScript(
      ({ workspace, language }) => {
        localStorage.setItem('lankide.workspace', JSON.stringify(workspace));
        localStorage.setItem('lankide.language', JSON.stringify(language));
      },
      { workspace, language },
    );
    await page.goto('/#/members');
    const card = page.getByTestId('lead-card');
    await card.click();
    const qr = card.locator('section[data-state]');
    await expect(qr).toHaveAttribute('data-state', 'wait');
    await rig({ op: 'feishu_scan', workspace, state: 'scanned' });
    await expect(qr).toHaveAttribute('data-state', 'scanned');
    await rig({ op: 'feishu_scan', workspace, state: 'complete' });
    await expect(qr).toHaveAttribute('data-state', 'error');
    await expect(qr.getByRole('alert')).toHaveText(
      language === 'en' ? 'Something went wrong. Try again.Try again' : '没有成功，请重试重试',
    );
    await expect(qr.getByRole('img')).toHaveCount(0);
    await page.screenshot({ path: testInfo.outputPath('owner-sign-in-error.png') });
    const retry = qr.getByRole('button', { name: /^(Try again|重试)$/ });
    await expect(retry).toBeVisible();
    // Closing clears the server candidate; a late poll cannot resume it.
    const cancelled = page.waitForResponse(
      (response) =>
        response.url().endsWith('/api/v1/invoke_provider_interface') &&
        (response.request().postData() ?? '').includes('switch_cancel'),
    );
    await qr.getByRole('button', { name: /^(Close|关闭)$/ }).click();
    expect((await cancelled).ok()).toBe(true);
    await expect(qr).toHaveCount(0);
    expect(JSON.parse((await invoke('switch_poll')).output_json).state).toBe('required');
    await card.click();
    await expect(qr).toHaveAttribute('data-state', 'error');
    // The same selected App is repaired by the real Provider after restart.
    await rig({ op: 'feishu_app_configuration', workspace, denied: false });
    await rig({ op: 'feishu_authorization', workspace, revoked: false });
    await rig({ op: 'feishu_scan', workspace, state: 'waiting' });
    await retry.click();
    await expect(qr).toHaveAttribute('data-state', 'wait');
    await rig({ op: 'feishu_scan', workspace, state: 'complete' });
    await expect(card.getByRole('article')).toBeVisible();
    await expect(card).not.toHaveAttribute('data-muted', 'true');
    await expect(qr).toHaveCount(0);
  });
}
