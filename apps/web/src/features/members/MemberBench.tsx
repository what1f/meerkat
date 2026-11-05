import { useState, type ReactNode } from 'react';
import { InlineText, useToast } from '../../design';
import { Badge, Bench, Board, Hang, QrTag, blank, type SheetClose } from '../../scene';
import { useT } from '../../lib/i18n';
import { agentStatus, useDeleteAgent, useResumeAgent } from '../../lib/queries/agents';
import { useWorkspace } from '../../lib/queries/workspaces';
import { useMemberDraft } from './useMemberDraft';
import { MemberConsole } from './MemberConsole';
import { AvatarPicker } from './AvatarPicker';
import { DeleteMemberDialog } from './DeleteMemberDialog';
import { MemberUnread } from './MemberUnread';
import { useModelFace } from './MemberCard';
import css from './MemberBench.module.css';

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
  const face = useModelFace(workspace, m.draft.machine_id, m.draft.harness_id, m.draft.model);
  const resume = useResumeAgent();
  const remove = useDeleteAgent();
  const [reading, setReading] = useState(false);
  const [picking, setPicking] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [swing, setSwing] = useState(0);
  const ref = id ? { workspace_id: workspace, agent_id: id } : null;
  const status = m.agent ? agentStatus(m.agent, m.machine, team.data).status : undefined;
  const { creation } = m;
  const channel = team.data?.provider_id === 'telegram' ? 'telegram' : 'feishu';
  const canCreate =
    !id &&
    (creation.step === 'idle' || creation.step === 'failed') &&
    m.draft.name.trim() !== '' &&
    !!m.draft.machine_id;

  const fail = () => toast({ message: t('members.failed') });
  const commit = async () => {
    try {
      if (id) {
        await m.save();
        setSwing((n) => n + 1);
        return true;
      }
      const created = await m.add();
      if (created) onCreated(created);
      return !!created;
    } catch {
      fail();
      return false;
    }
  };
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
      <Bench {...blank}>
        <MemberConsole
          m={m}
          workspace={workspace}
          channel={channel}
          isNew={!id}
          status={status}
          reading={reading}
          onReading={setReading}
          canCreate={canCreate}
          onCommit={() => void commit()}
          onResume={() =>
            ref && resume.mutate({ request_id: crypto.randomUUID(), agent: ref }, { onError: fail })
          }
          onDelete={() => setConfirming(true)}
          onOpen={(created) =>
            // A failed upload keeps this page and the picked avatar, so opening can be tried again.
            void m.finish().then((done) => done && onCreated(created), fail)
          }
        />
        <div className={css.stage} {...blank}>
          <Hang swingKey={swing} faded={reading}>
            <AvatarPicker
              open={picking}
              onOpenChange={setPicking}
              onPick={(upload) => {
                m.setUpload(upload);
                setPicking(false);
                setSwing((n) => n + 1);
              }}
              trigger={
                <Badge
                  size="stage"
                  dimmed={!!m.agent?.pause_reason}
                  foot={
                    <>
                      {face.mark}
                      {face.label}
                    </>
                  }
                  avatar={{
                    src: m.avatarSrc,
                    status,
                    statusTip: m.agent?.pause_reason ?? undefined,
                    creating: ['creating', 'confirm', 'permissions'].includes(creation.step),
                    onEdit: () => setPicking(true),
                    editLabel: t('members.avatar'),
                  }}
                  name={
                    <InlineText
                      mode="live"
                      value={m.draft.name}
                      onChange={(name) => m.set({ name })}
                      placeholder={t('members.name')}
                      textStyle="heading"
                      autoFocus={!id}
                      data-testid="member-name"
                    />
                  }
                  roleText={m.draft.role}
                  role={
                    <InlineText
                      mode="live"
                      value={m.draft.role}
                      onChange={(role) => m.set({ role })}
                      placeholder={t('members.role')}
                      textStyle="small"
                    />
                  }
                  bio={
                    <InlineText
                      mode="live"
                      multiline
                      value={m.draft.bio}
                      onChange={(bio) => m.set({ bio })}
                      placeholder={t('members.bio')}
                      textStyle="small"
                    />
                  }
                />
              }
            />
          </Hang>
          {(creation.step === 'confirm' || creation.step === 'permissions') && (
            <QrTag
              state="wait"
              value={creation.step === 'confirm' ? creation.qr : creation.url}
              brand={channel}
              // Codes are renewed by the onboarding driver; there is nothing to refresh by hand yet.
              onRefresh={() => {}}
              refreshLabel={t('members.refreshQr')}
              data-testid="member-create-qr"
            />
          )}
        </div>
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
      </Bench>
    </Board>
  );
  return frame({ close }, body);
}
