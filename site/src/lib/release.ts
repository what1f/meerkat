export type AssetKey =
  | 'macos-arm64'
  | 'macos-x86_64'
  | 'linux-x86_64'
  | 'linux-arm64'
  | 'node-macos-arm64'
  | 'node-macos-x86_64'
  | 'install-sh';

export interface AssetInfo {
  name: string;
  url: string;
  bytes: number;
}

export interface ReleaseInfo {
  tag: string;
  version: string;
  htmlUrl: string;
  assets: Record<AssetKey, AssetInfo>;
}

const REPO = 'https://github.com/what1f/meerkat';

export const FALLBACK_VERSION = '0.1.0';

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '';
  if (bytes < 1024 * 1024) return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function buildFallbackRelease(version = FALLBACK_VERSION): ReleaseInfo {
  const tag = version.startsWith('v') ? version : `v${version}`;
  const clean = tag.slice(1);
  const dl = (name: string) => `${REPO}/releases/download/${tag}/${name}`;
  const asset = (name: string, bytes: number): AssetInfo => ({ name, url: dl(name), bytes });

  return {
    tag,
    version: clean,
    htmlUrl: `${REPO}/releases/tag/${tag}`,
    assets: {
      'macos-arm64': asset(`Meerkat-${clean}-macos-arm64.dmg`, 34721425),
      'macos-x86_64': asset(`Meerkat-${clean}-macos-x86_64.dmg`, 36795834),
      'linux-x86_64': asset(`meerkat-machine-${clean}-linux-x86_64.tar.gz`, 11565957),
      'linux-arm64': asset(`meerkat-machine-${clean}-linux-arm64.tar.gz`, 10965321),
      'node-macos-arm64': asset(`meerkat-machine-${clean}-macos-arm64.tar.gz`, 10800062),
      'node-macos-x86_64': asset(`meerkat-machine-${clean}-macos-x86_64.tar.gz`, 11849414),
      'install-sh': asset('install.sh', 3498),
    },
  };
}

interface GitHubAssetPayload {
  name?: unknown;
  browser_download_url?: unknown;
  size?: unknown;
}

interface GitHubReleasePayload {
  tag_name?: unknown;
  html_url?: unknown;
  assets?: unknown;
}

const MATCHERS: [AssetKey, RegExp][] = [
  ['macos-arm64', /^Meerkat-.*-macos-arm64\.dmg$/],
  ['macos-x86_64', /^Meerkat-.*-macos-x86_64\.dmg$/],
  ['linux-x86_64', /^meerkat-machine-.*-linux-x86_64\.tar\.gz$/],
  ['linux-arm64', /^meerkat-machine-.*-linux-arm64\.tar\.gz$/],
  ['node-macos-arm64', /^meerkat-machine-.*-macos-arm64\.tar\.gz$/],
  ['node-macos-x86_64', /^meerkat-machine-.*-macos-x86_64\.tar\.gz$/],
  ['install-sh', /^install\.sh$/],
];

export function parseGitHubRelease(payload: unknown): ReleaseInfo | null {
  if (!payload || typeof payload !== 'object') return null;
  const data = payload as GitHubReleasePayload;
  if (typeof data.tag_name !== 'string' || !data.tag_name.trim()) return null;
  if (!Array.isArray(data.assets)) return null;

  const tag = data.tag_name.trim();
  const version = tag.replace(/^v/, '');
  const base = buildFallbackRelease(version);
  const next: Record<AssetKey, AssetInfo> = { ...base.assets };
  let matched = 0;

  for (const raw of data.assets) {
    if (!raw || typeof raw !== 'object') continue;
    const item = raw as GitHubAssetPayload;
    if (typeof item.name !== 'string' || typeof item.browser_download_url !== 'string') continue;
    const size = typeof item.size === 'number' ? item.size : 0;
    for (const [key, pattern] of MATCHERS) {
      if (pattern.test(item.name)) {
        next[key] = { name: item.name, url: item.browser_download_url, bytes: size };
        matched += 1;
        break;
      }
    }
  }

  if (matched === 0) return null;

  return {
    tag,
    version,
    htmlUrl: typeof data.html_url === 'string' && data.html_url ? data.html_url : `${REPO}/releases/tag/${tag}`,
    assets: next,
  };
}

export function installCommand(installShUrl: string): string {
  return `curl -fsSL ${installShUrl} | sh`;
}
