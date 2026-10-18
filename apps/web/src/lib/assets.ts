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
export const avatarUrl = (id: string) => assetUrl(`avatars/${id}.jpg`);
export const figureUrl = (id: string, pose: 'work' | 'sleep' | 'carry' | 'stand') =>
  assetUrl(`figures/${id}/${pose}.png`);
export const reactionUrl = (id: string) => assetUrl(`reactions/${id}.png`);
const brandPaths: Record<string, string> = {
  codex: 'lobehub/codex',
  'claude-code': 'lobehub/claude-code',
  pi: 'lobehub/pi',
};
export const brandUrl = (id: string) => assetUrl(`brand/${brandPaths[id] ?? id}.svg`);
