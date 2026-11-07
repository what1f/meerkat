import { useState, type ReactNode } from 'react';
import { useToast } from '../../design';
import { Board, type SheetClose } from '../../scene';
import { useT } from '../../lib/i18n';
import { agentStatus, useDeleteAgent, useResumeAgent } from '../../lib/queries/agents';
import { useWorkspace } from '../../lib/queries/workspaces';
import { useMemberDraft } from './useMemberDraft';
import { MemberStage } from './MemberStage';
import { DeleteMemberDialog } from './DeleteMemberDialog';
import { MemberUnread } from './MemberUnread';

export type TopBarConfig = { title?: string; close?: SheetClose };

export function MemberBench({
  workspace,
  id,
  onBack,
  onCreated,
  frame,
}: {
  workspace: string;
  id: string | null;
  onBack: () => void;
  onCreated: (id: string) => void;
  frame: (bar: TopBarConfig, body: ReactNode, board?: 'wall' | 'open') => ReactNode;
}) {
  const t = useT();
  const toast = useToast();
  const team = useWorkspace({ request_id: 'member', workspace: { workspace_id: workspace } });
  const m = useMemberDraft(workspace, id, team.data?.provider_id);
  const resume = useResumeAgent();
  const remove = useDeleteAgent();
  const [confirming, setConfirming] = useState(false);
  const ref = id ? { workspace_id: workspace, agent_id: id } : null;
  const status = m.agent ? agentStatus(m.agent, m.machine, team.data).status : undefined;
  const { creation } = m;
  const channel = team.data?.provider_id === 'telegram' ? 'telegram' : 'feishu';
  const fail = () => toast({ message: t('members.failed') });
  const home = { onClose: onBack };
  const unsaved = id ? m.dirty : creation.step === 'idle' && !!m.draft.name.trim();
  // Unsaved edits keep the member out; the lit key saves them, the toast lets them go.
  const close: SheetClose = unsaved
    ? {
        onClose: onBack,
        blocked: () =>
          toast({
            message: t('members.unsaved'),
            action: { label: t('members.discard'), onAction: onBack },
          }),
      }
    : home;

  if (m.notFound || m.failed || m.loading) return frame({ close: home }, <MemberUnread m={m} />);
  const body = (
    <Board layout="open">
      <MemberStage
        m={m}
        workspace={workspace}
        id={id}
        channel={channel}
        status={status}
        onCreated={onCreated}
        onResume={() =>
          ref && resume.mutate({ request_id: crypto.randomUUID(), agent: ref }, { onError: fail })
        }
        onDelete={() => setConfirming(true)}
      />
      <DeleteMemberDialog
        open={confirming}
        name={m.draft.name}
        avatar={m.avatarSrc}
        onClose={() => setConfirming(false)}
        pending={remove.isPending}
        onConfirm={() =>
          ref &&
          !remove.isPending &&
          remove.mutate(
            { request_id: crypto.randomUUID(), agent: ref },
            { onSuccess: onBack, onError: fail, onSettled: () => setConfirming(false) },
          )
        }
      />
    </Board>
  );
  return frame({ close }, body);
}
