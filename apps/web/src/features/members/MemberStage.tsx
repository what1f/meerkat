// eslint-disable-next-line no-restricted-imports -- Opening the member follows the creation the server finishes.
import { useEffect, useRef, useState } from 'react';
import { BrandMark, InlineText, LinkButton, useToast, type Status } from '../../design';
import { Badge, Bench, Hang, QrTag, Scribble, blank } from '../../scene';
import { useT } from '../../lib/i18n';
import type { useMemberDraft } from './useMemberDraft';
import { MemberConsole } from './MemberConsole';
import { AvatarPicker } from './AvatarPicker';
import { useModelFace } from './MemberCard';
import css from './MemberBench.module.css';

type MemberDraftState = ReturnType<typeof useMemberDraft>;

// Feishu opens the link inside its own app; Telegram links open it already.
const inApp = (channel: 'feishu' | 'telegram', link: string) =>
  channel === 'feishu'
    ? `https://applink.feishu.cn/client/web_url/open?mode=window&url=${encodeURIComponent(link)}`
    : link;

// The console with the badge hanging beside it. A new member is created in place:
// the chat app's code hangs beside the badge, with a note on what to do with it.
export function MemberStage({
  m,
  workspace,
  id,
  channel,
  status,
  onCreated,
  autoOpen,
  onResume,
  onDelete,
}: {
  m: MemberDraftState;
  workspace: string;
  id: string | null;
  channel: 'feishu' | 'telegram';
  status?: Status;
  onCreated: (id: string) => void;
  /** Opens the member as soon as they have joined, instead of offering to. */
  autoOpen?: boolean;
  onResume: () => void;
  onDelete: () => void;
}) {
  const t = useT();
  const toast = useToast();
  const face = useModelFace(workspace, m.draft.machine_id, m.draft.harness_id, m.draft.model);
  const [reading, setReading] = useState(false);
  const [picking, setPicking] = useState(false);
  const [swing, setSwing] = useState(0);
  const { creation } = m;
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
        return;
      }
      const created = await m.add();
      if (created) onCreated(created);
    } catch {
      fail();
    }
  };
  // A failed upload keeps this page and the picked avatar, so opening can be tried again.
  const open = (created: string) =>
    void m.finish().then((done) => done && onCreated(created), fail);
  const opened = useRef(false);
  // eslint-disable-next-line no-restricted-syntax -- The server finishes the creation; opening follows it once.
  useEffect(() => {
    if (!autoOpen || creation.step !== 'ready' || opened.current) return;
    opened.current = true;
    open(creation.id);
  });
  const code =
    creation.step === 'confirm' ? creation.qr : creation.step === 'permissions' ? creation.url : '';
  const feishu = channel === 'feishu';
  const note =
    creation.step === 'permissions'
      ? t('members.scanAgain')
      : t(feishu ? 'members.scanFeishu' : 'members.scanTelegram');
  return (
    <Bench {...blank}>
      <MemberConsole
        m={m}
        workspace={workspace}
        isNew={!id}
        status={status}
        reading={reading}
        onReading={setReading}
        canCreate={canCreate}
        autoOpen={autoOpen}
        onCommit={() => void commit()}
        onResume={onResume}
        onDelete={onDelete}
        onOpen={open}
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
                  // Created and waiting on its permissions: the member is there already.
                  status: creation.step === 'permissions' ? 'ok' : status,
                  statusTip: m.agent?.pause_reason ?? undefined,
                  creating: ['creating', 'confirm'].includes(creation.step),
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
        {code && (
          <div className={css.code}>
            <QrTag
              key={creation.step}
              state="wait"
              value={code}
              brand={channel}
              data-phase={creation.step}
              // Codes are renewed by the onboarding driver; there is nothing to refresh by hand yet.
              onRefresh={() => {}}
              refreshLabel={t('members.refreshQr')}
              data-testid="member-create-qr"
            />
            <Scribble key={note} arrow="up" text={note} />
            <LinkButton
              data-testid="member-open-app"
              onClick={() => window.open(inApp(channel, code), '_blank', 'noopener')}
            >
              <BrandMark brand={channel} size={14} />
              {t(feishu ? 'members.openFeishu' : 'members.openTelegram')}
            </LinkButton>
          </div>
        )}
      </div>
    </Bench>
  );
}
