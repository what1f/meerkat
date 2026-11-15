import { buildFallbackRelease, parseGitHubRelease, type ReleaseInfo } from './lib/release';

// Links that change once the repository is public.
export const repo = 'https://github.com/what1f/meerkat';
export const latestReleaseApi = 'https://api.github.com/repos/what1f/meerkat/releases/latest';

export async function getLatestRelease(): Promise<ReleaseInfo> {
  const fallback = buildFallbackRelease();
  try {
    const response = await fetch(latestReleaseApi, {
      headers: { Accept: 'application/vnd.github+json' },
      signal: AbortSignal.timeout(2500),
    });
    if (!response.ok) return fallback;
    return parseGitHubRelease(await response.json()) ?? fallback;
  } catch {
    return fallback;
  }
}
