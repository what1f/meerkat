import type { FigurePack } from './api/generated';
const assets = import.meta.glob<string>('../assets/**/*', {
  eager: true,
  query: '?url',
  import: 'default',
});
export function assetUrl(path: string): string | undefined {
  return assets[`../assets/${path}`];
}
// Resource IDs are scoped to a workspace.
export const resourceUrl = (id: number, workspaceId: string): string =>
  `/api/v1/resources/${id}?workspace=${encodeURIComponent(workspaceId)}`;
// Imported looks are served as Resources; the scene draws them from these URLs.
export const packFrames = (pack: FigurePack, workspaceId: string) => ({
  scale: pack.scale,
  frames: {
    work: resourceUrl(pack.frames.work.resource_id, workspaceId),
    sleep: resourceUrl(pack.frames.sleep.resource_id, workspaceId),
    carry: resourceUrl(pack.frames.carry.resource_id, workspaceId),
    stand: resourceUrl(pack.frames.stand.resource_id, workspaceId),
  },
});
// The Runtime stores a built-in look as `builtin:<id>`; `default` and pack ids pass through.
export const figureId = (value: string) =>
  value.startsWith('builtin:') ? value.slice('builtin:'.length) : value;
export const avatarUrl = (id: string) => assetUrl(`avatars/${id}.jpg`);
export const figureUrl = (id: string, pose: 'work' | 'sleep' | 'carry' | 'stand') =>
  assetUrl(`figures/${id}/${pose}.png`);
export const reactionUrl = (id: string) => assetUrl(`reactions/${id}.png`);
const brandPaths: Record<string, string> = {
  codex: 'lobehub/codex',
  claude_code: 'lobehub/claude-code',
  pi: 'lobehub/pi',
};
export const brandUrl = (id: string) => assetUrl(`brand/${brandPaths[id] ?? id}.svg`);
