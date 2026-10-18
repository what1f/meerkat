import { useState } from 'react';
import { BrandMark } from '../../design';
import { Badge, BadgeVerso, FlipCard, Hang, QrTag } from '../../scene';
import { defaultAvatar } from './avatars';
import { useT } from '../../lib/i18n';
import { avatarUrl, resourceUrl } from '../../lib/assets';
import { useRefreshWorkspaceOwner, useWorkspaceOwner } from '../../lib/queries/workspaces';
import {
  useCancelFeishuAccountSwitch,
  useFeishuAccountSwitch,
  useStartFeishuAccountSwitch,
} from '../../lib/queries/onboarding';

// Your own badge. The profile comes from the chat account, so nothing here is edited.
export function LeadCard({
  workspace,
  provider,
}: {
  workspace: string;
  provider: 'feishu' | 'telegram';
}) {
  const t = useT();
  const request = { request_id: 'lead', workspace: { workspace_id: workspace } };
  const owner = useWorkspaceOwner({ ...request, provider });
  const refresh = useRefreshWorkspaceOwner();
  const builtin = avatarUrl(defaultAvatar(workspace));
  const [flipped, setFlipped] = useState(false);
  const [switching, setSwitching] = useState(false);
  const account = useFeishuAccountSwitch(request, switching);
  const start = useStartFeishuAccountSwitch();
  const cancel = useCancelFeishuAccountSwitch();
  const scan = account.data?.state;
  const showQr = switching && (start.isPending || scan === 'waiting' || scan === 'scanned');

  const actions = [
    {
      label: t('members.sync'),
      busy: refresh.isPending,
      onClick: () => refresh.mutate({ ...request, provider }),
    },
  ];
  if (provider === 'feishu')
    actions.push({
      label: t('members.switchAccount'),
      busy: false,
      onClick: () => {
        setFlipped(false);
        setSwitching(true);
        start.mutate(request);
      },
    });

  if (showQr)
    return (
      <Hang lanyard="lead" data-testid="lead-card">
        <QrTag
          state={
            start.isPending || !account.data?.qr_code
              ? 'loading'
              : scan === 'scanned'
                ? 'scanned'
                : 'wait'
          }
          value={account.data?.qr_code ?? ''}
          brand="feishu"
          onRefresh={() => start.mutate(request)}
          refreshLabel={t('members.refreshQr')}
          onKeyDown={(e) => {
            if (e.key !== 'Escape') return;
            setSwitching(false);
            cancel.mutate(request);
          }}
        />
      </Hang>
    );
  return (
    <Hang lanyard="lead" motion={switching ? 'turn-in' : undefined} data-testid="lead-card">
      <FlipCard
        flipped={flipped}
        onFlip={() => setFlipped(!flipped)}
        front={
          <Badge
            size="card"
            avatar={{
              // Without a picture of its own, the owner wears the same built-in one as on a failed load.
              src: owner.data?.avatar
                ? resourceUrl(owner.data.avatar.resource_id, workspace)
                : owner.isPending
                  ? undefined
                  : builtin,
              fallback: builtin,
            }}
            name={owner.data?.name ?? t('members.you')}
            ghost={owner.isPending}
          />
        }
        back={<BadgeVerso mark={<BrandMark brand={provider} />} actions={actions} />}
      />
    </Hang>
  );
}
