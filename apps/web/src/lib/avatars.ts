import { avatarUrl } from './assets';
import type * as DTO from './api/generated';

export const builtinAvatars = Array.from({ length: 17 }, (_, i) => String(i).padStart(2, '0'));

// Every new member starts with a built-in avatar picked from its id, so it never shows blank.
export function defaultAvatar(agentId: string) {
  let hash = 0;
  for (const char of agentId) hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
  return builtinAvatars[hash % builtinAvatars.length]!;
}

export async function toUpload(blob: Blob): Promise<DTO.AgentAvatarUpload> {
  const url = await new Promise<string>((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(blob);
  });
  return { media_type: blob.type || 'image/jpeg', bytes: url.slice(url.indexOf(',') + 1) };
}

// A built-in avatar is stored like an uploaded one.
export async function builtinUpload(id: string) {
  const url = avatarUrl(id);
  return url ? toUpload(await (await fetch(url)).blob()) : null;
}

export const avatarChoices = builtinAvatars.map((id) => ({ id, src: avatarUrl(id) }));

// What the avatar picker returned, ready to send.
export const choiceUpload = (choice: { builtin: string } | { file: File }) =>
  'builtin' in choice ? builtinUpload(choice.builtin) : toUpload(choice.file);
