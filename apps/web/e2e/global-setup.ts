import { startRig } from './rig-process';

export default async function setup() {
  const running = await startRig();
  process.env.LANKIDE_WEB_CONTROL_PORT = String(running.controlPort);
  process.env.LANKIDE_WEB_BASE_URL = running.baseURL;
  return running.cleanup;
}
