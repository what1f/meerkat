export type AssetKey =
  | 'macos-arm64'
  | 'macos-x86_64'
  | 'windows-x86_64'
  | 'linux-x86_64'
  | 'linux-arm64';

export interface ReleaseInfo {
  tag: string;
  htmlUrl: string;
  assets: Partial<Record<AssetKey, string>>;
}

const REPO = 'https://github.com/what1f/meerkat';
export const FALLBACK_VERSION = '0.2.0';

export function buildFallbackRelease(version = FALLBACK_VERSION): ReleaseInfo {
  const tag = version.startsWith('v') ? version : `v${version}`;
  const clean = tag.slice(1);
  return {
    tag,
    htmlUrl: `${REPO}/releases/tag/${tag}`,
    assets: {
      'macos-arm64': `${REPO}/releases/download/${tag}/Meerkat-${clean}-macos-arm64.dmg`,
      'macos-x86_64': `${REPO}/releases/download/${tag}/Meerkat-${clean}-macos-x86_64.dmg`,
      'windows-x86_64': `${REPO}/releases/download/${tag}/Meerkat-${clean}-windows-x86_64-setup.exe`,
      'linux-x86_64': `${REPO}/releases/download/${tag}/Meerkat-${clean}-linux-x86_64.tar.gz`,
      'linux-arm64': `${REPO}/releases/download/${tag}/Meerkat-${clean}-linux-arm64.tar.gz`,
    },
  };
}

export function parseGitHubRelease(payload: unknown): ReleaseInfo | null {
  if (!payload || typeof payload !== 'object') return null;
  const data = payload as { tag_name?: unknown; html_url?: unknown; assets?: unknown };
  if (typeof data.tag_name !== 'string' || !data.tag_name.trim() || !Array.isArray(data.assets)) return null;

  const tag = data.tag_name.trim();
  const assets: Partial<Record<AssetKey, string>> = {};
  let matched = 0;

  for (const item of data.assets) {
    if (!item || typeof item !== 'object') continue;
    const { name, browser_download_url } = item as { name?: unknown; browser_download_url?: unknown };
    if (typeof name !== 'string' || typeof browser_download_url !== 'string') continue;
    if (/^Meerkat-.*-macos-arm64\.dmg$/.test(name)) {
      assets['macos-arm64'] = browser_download_url;
      matched += 1;
    } else if (/^Meerkat-.*-macos-x86_64\.dmg$/.test(name)) {
      assets['macos-x86_64'] = browser_download_url;
      matched += 1;
    } else if (/^Meerkat-.*-windows-x86_64-setup\.exe$/.test(name)) {
      assets['windows-x86_64'] = browser_download_url;
      matched += 1;
    } else if (/^Meerkat-.*-linux-x86_64\.tar\.gz$/.test(name)) {
      assets['linux-x86_64'] = browser_download_url;
      matched += 1;
    } else if (/^Meerkat-.*-linux-arm64\.tar\.gz$/.test(name)) {
      assets['linux-arm64'] = browser_download_url;
      matched += 1;
    }
  }

  if (matched === 0) return null;
  return {
    tag,
    htmlUrl: typeof data.html_url === 'string' && data.html_url ? data.html_url : `${REPO}/releases/tag/${tag}`,
    assets,
  };
}
