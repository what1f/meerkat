import { spawn, execFileSync } from 'node:child_process';
import { once } from 'node:events';
import { rm, readFile, readdir } from 'node:fs/promises';
import { setTimeout as delay } from 'node:timers/promises';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';

type Ready = {
  baseURL: string;
  controlPort: number;
  runtimeHome: string;
  machineHome: string;
};

export async function startRig() {
  const root = fileURLToPath(new URL('../../../', import.meta.url));
  const metadata = JSON.parse(
    execFileSync('cargo', ['metadata', '--no-deps', '--format-version', '1'], {
      cwd: root,
      encoding: 'utf8',
    }),
  ) as { target_directory: string };
  const child = spawn(join(metadata.target_directory, 'debug/system-web-rig'), [], {
    cwd: root,
    stdio: ['pipe', 'pipe', 'inherit'],
    detached: true,
  });
  const exited = once(child, 'exit');
  const lines = createInterface({ input: child.stdout });
  let ready: Ready | undefined;
  const cleanup = async () => {
    lines.close();
    child.stdin.end();
    let timer: ReturnType<typeof setTimeout> | undefined;
    let timedOut = false;
    try {
      await Promise.race([
        exited,
        new Promise<void>((resolve) => {
          timer = setTimeout(() => {
            timedOut = true;
            resolve();
          }, 20_000);
        }),
      ]);
    } finally {
      clearTimeout(timer);
      const signal = (pid: number) => {
        try {
          process.kill(-pid, 'SIGKILL');
        } catch (error) {
          if ((error as NodeJS.ErrnoException).code !== 'ESRCH') throw error;
        }
      };
      const owned = child.pid
        ? execFileSync('ps', ['-axo', 'pid=,pgid='], { encoding: 'utf8' })
            .trim()
            .split('\n')
            .flatMap((row) => {
              const [pid, group] = row.trim().split(/\s+/).map(Number);
              return group === child.pid ? [pid!] : [];
            })
        : [];
      if (child.pid) signal(child.pid);
      await Promise.race([
        exited,
        delay(5_000, undefined, { ref: false }).then(() => {
          throw new Error('Rig did not exit after SIGKILL; homes retained');
        }),
      ]);
      if (ready) {
        const homes = [ready.machineHome];
        const remoteRoot = join(ready.runtimeHome, 'web-machines');
        for (const entry of await readdir(remoteRoot, { withFileTypes: true }).catch(() => [])) {
          if (entry.isDirectory()) homes.push(join(remoteRoot, entry.name));
        }
        const peers: number[] = [];
        for (const home of homes) {
          const journal = await readFile(join(home, 'harness-pids.tsv'), 'utf8').catch(() => '');
          for (const line of journal.split('\n')) {
            const match = /^spawned\t(\d+)$/.exec(line);
            if (match) peers.push(Number(match[1]));
          }
        }
        for (const pid of peers) signal(pid);
        // Harness groups are independent from the rig group. Keep their journals
        // until every owned process has exited, including the SIGSTOP timeout case.
        const alive = (pid: number) => {
          try {
            const state = execFileSync('ps', ['-o', 'stat=', '-p', String(pid)], {
              encoding: 'utf8',
            }).trim();
            return state !== '' && !state.startsWith('Z');
          } catch {
            return false;
          }
        };
        const deadline = Date.now() + 5_000;
        while ([...owned, ...peers].some(alive)) {
          if (Date.now() >= deadline)
            throw new Error('Owned rig processes did not exit; homes retained');
          await delay(50);
        }
        await Promise.all([
          rm(ready.runtimeHome, { recursive: true, force: true }),
          rm(ready.machineHome, { recursive: true, force: true }),
        ]);
      }
    }
    if (timedOut) throw new Error('Web rig shutdown timed out');
  };
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    ready = await Promise.race([
      new Promise<Ready>((resolve, reject) => {
        lines.on('line', (line) => {
          try {
            const candidate = JSON.parse(line) as Ready;
            if (typeof candidate.baseURL === 'string' && typeof candidate.controlPort === 'number')
              resolve(candidate);
          } catch {
            // Daemon diagnostics can also reach stdout; readiness is the URL record.
          }
        });
        child.once('error', reject);
        timer = setTimeout(() => reject(new Error('Web rig startup timed out')), 30_000);
      }),
      exited.then(([code]) => {
        throw new Error(`Web rig exited before startup: ${code}`);
      }),
    ]);
    return { ...ready, pid: child.pid!, cleanup };
  } catch (error) {
    await cleanup();
    throw error;
  } finally {
    clearTimeout(timer);
  }
}
