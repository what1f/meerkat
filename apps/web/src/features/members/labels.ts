import { t as translate, type MessageKey } from '../../lib/i18n';

// The Built-in Agent is ours, so its name is translated instead of taken from the Machine.
export const harnessName = (
  h: { harness_id: string; display_name: string } | undefined,
  id: string,
) => (id === 'builtin' ? translate('builtinAgent.name') : (h?.display_name ?? id));

export const levelLabel = (level: string) => {
  const key = `members.levels.${level}` as MessageKey;
  try {
    return translate(key);
  } catch {
    return level;
  }
};

// The same name from different sources is told apart by the source; unique names stay bare.
export const named = (x: { model_id: string; display_name: string }) =>
  x.display_name || x.model_id;
export const sourceOf = (id: string) => (id.includes('/') ? id.slice(0, id.indexOf('/')) : id);
