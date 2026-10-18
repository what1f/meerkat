import { expect, request } from '@playwright/test';
import { rig } from './rig';

export async function api(baseURL: string, op: string, body: object = {}) {
  const client = await request.newContext({ baseURL });
  try {
    const response = await client.post(`/api/v1/${op}`, { data: { request_id: op, ...body } });
    expect(response.ok(), await response.text()).toBe(true);
    return await response.json();
  } finally {
    await client.dispose();
  }
}
export async function signInFeishu(baseURL: string, workspace: string) {
  await rig({ op: 'create_workspace', workspace, provider: 'feishu' });
  const login = (action: string) =>
    api(baseURL, 'invoke_provider_interface', {
      workspace: { workspace_id: workspace },
      interface_id: 'feishu.workspace.console_login',
      input_json: JSON.stringify({ action }),
    });
  await rig({ op: 'feishu_scan', workspace, state: 'waiting' });
  await login('start');
  await rig({ op: 'feishu_scan', workspace, state: 'complete' });
  expect(JSON.parse((await login('poll')).output_json).state).toBe('ready');
}
// Each new bot enrolls its authenticated creator.
export async function addFeishuMember(baseURL: string, workspace: string, agent = 'alpha') {
  await rig({ op: 'create_agent', workspace, agent });
  let progress;
  for (const action of ['start', 'poll', 'start'])
    progress = await api(baseURL, 'advance_agent_onboarding', {
      agent: { workspace_id: workspace, agent_id: agent },
      action,
      field_values_json: progress?.payload_json ?? '{}',
    });
  expect(progress.status).toBe('ready');
}
