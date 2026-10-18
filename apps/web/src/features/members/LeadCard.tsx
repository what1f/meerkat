import { useState } from 'react';
import { BrandMark, useToast } from '../../design';
import { Badge, BadgeVerso, FlipCard, Hang, QrTag } from '../../scene';
import { defaultAvatar } from './avatars';
import { ApiError } from '../../lib/api/client';
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
  const toast = useToast();
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
  // Without a picture of its own, the owner wears the same built-in one as on a failed load.
  const ownerPicture = owner.data?.avatar
    ? resourceUrl(owner.data.avatar.resource_id, workspace)
    : owner.isPending
      ? undefined
      : builtin;
  const needsSignIn = provider === 'feishu' && owner.data?.state === 'needs_sign_in';
  const showQr = switching && (start.isPending || scan === 'waiting' || scan === 'scanned');

  const begin = () => {
    setFlipped(false);
    setSwitching(true);
    start.mutate(request, {
      onError: (error) => {
        setSwitching(false);
        toast({
          message: t(
            error instanceof ApiError && error.detail === 'owner_app_unavailable'
              ? 'members.signInUnavailable'
              : 'members.failed',
          ),
        });
      },
    });
  };
  const close = () => {
    setSwitching(false);
    cancel.mutate(request);
  };

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
      onClick: begin,
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
          onClose={close}
          closeLabel={t('members.closeQr')}
          onKeyDown={(e) => e.key === 'Escape' && close()}
        />
      </Hang>
    );
  if (needsSignIn)
    return (
      <Hang
        lanyard="lead"
        muted
        data-testid="lead-card"
        aria-label={t('members.signIn')}
        onClick={begin}
      >
        <Badge
          size="card"
          avatar={{ src: ownerPicture, fallback: builtin, status: 'err' }}
          name={owner.data?.name ?? t('members.you')}
          dimmed
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
              src: ownerPicture,
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
