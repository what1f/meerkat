import { Badge, Hang } from '../../scene';
import { useT } from '../../lib/i18n';
import { useWorkspaceOwner } from '../../lib/queries/workspaces';
import type { Provider } from './useOnboarding';

// Your own badge. Feishu has signed you in by now; Telegram knows you only after
// you confirm the first member, so its badge stays blank until then.
export function OwnerBadge({ workspace, provider }: { workspace?: string; provider: Provider }) {
  const t = useT();
  const owner = useWorkspaceOwner(
    { request_id: 'onboarding', workspace: { workspace_id: workspace ?? '' }, provider },
    !!workspace,
  ).data;
  const known = owner?.state === 'ready' && !!owner.name;
  const blank = !known && provider === 'telegram';
  return (
    <Hang lanyard="lead" data-testid="onboarding-owner" data-state={known ? 'ready' : 'unknown'}>
      <Badge
        size="card"
        ghost={blank}
        avatar={{}}
        name={known ? owner.name : blank ? '' : t('onboarding.you')}
      />
    </Hang>
  );
}
