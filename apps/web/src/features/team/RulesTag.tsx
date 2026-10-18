import { useState } from 'react';
import { DocArea, useToast } from '../../design';
import { Tag } from '../../scene';
import { useT } from '../../lib/i18n';
import { useUpdateWorkspace } from '../../lib/queries/workspaces';
import type { Workspace } from '../../lib/api/generated';
import './messages';

// The team's rules on a paper tag, saved when the field loses focus.
export function RulesTag({ workspace }: { workspace: Workspace }) {
  const t = useT();
  const toast = useToast();
  // Saves run in order, so the last edit is the one that stays.
  const update = useUpdateWorkspace(`rules-${workspace.workspace?.workspace_id}`);
  const [draft, setDraft] = useState<string | null>(null);
  const save = () => {
    if (draft === null) return;
    // A queued save is what the rules will become, so compare with that rather than the cache.
    if (update.isPending) {
      if (draft === update.variables.rules) return;
    } else if (draft === workspace.rules) return setDraft(null);
    const sent = draft;
    update.mutate(
      {
        request_id: 'update_workspace_rules',
        workspace: workspace.workspace,
        name: null,
        rules: sent,
        provider_config_json: null,
        cover_resource_id: null,
        provider_secrets_json: null,
      },
      {
        // Typing that continues while the save is in flight stays in the draft.
        onSuccess: () => setDraft((current) => (current === sent ? null : current)),
        onError: () => toast({ message: t('team.failed') }),
      },
    );
  };
  return (
    <Tag>
      <h6>{t('team.rules')}</h6>
      <DocArea
        data-testid="team-rules"
        aria-label={t('team.rules')}
        value={draft ?? workspace.rules}
        onChange={setDraft}
        onBlur={save}
        placeholder={t('team.rulesPlaceholder')}
      />
    </Tag>
  );
}
