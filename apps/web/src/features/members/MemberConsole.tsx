import type { ReactNode } from 'react';
import { Button, Callout, Icon, Spinner, type Status } from '../../design';
import { Console, ConsoleKey, SoulSlot, Sticker } from '../../scene';
import { useT } from '../../lib/i18n';
import type { useMemberDraft } from './useMemberDraft';
import { BrainPicker } from './BrainPicker';
import { FigurePicker } from './FigurePicker';
import { ReactionPicker } from './ReactionPicker';
import { AdvancedDrawer } from './AdvancedDrawer';
import css from './MemberBench.module.css';

export type MemberDraftState = ReturnType<typeof useMemberDraft>;

export function MemberConsole({
  m,
  workspace,
  isNew,
  status,
  reading,
  onReading,
  canCreate,
  onCommit,
  onResume,
  onDelete,
  onOpen,
}: {
  m: MemberDraftState;
  workspace: string;
  isNew: boolean;
  status: Status | undefined;
  reading: boolean;
  onReading: (reading: boolean) => void;
  canCreate: boolean;
  onCommit: () => void;
  onResume: () => void;
  onDelete: () => void;
  onOpen: (id: string) => void;
}) {
  const t = useT();
  const paused = m.agent?.pause_reason ?? null;
  const reaction = m.draft.auto_reaction || null;
  const pickReaction = (auto_reaction: string) => m.set({ auto_reaction });
  const { creation } = m;
  const creating: ReactNode =
    creation.step === 'confirm' ? (
      <Callout tone="quiet" leading={<Spinner />}>
        {t('members.scanToCreate')}
      </Callout>
    ) : creation.step === 'failed' ? (
      <Callout tone="attention">{t('members.failed')}</Callout>
    ) : creation.step === 'ready' ? (
      <Callout
        tone="quiet"
        action={
          <Button size="sm" variant="primary" onClick={() => onOpen(creation.id)}>
            {t('members.open')}
          </Button>
        }
      >
        {t('members.joined')}
      </Callout>
    ) : null;
  return (
    <Console
      reading={reading}
      notice={
        creating ??
        (paused ? (
          <Callout
            tone="quiet"
            leading={<Icon name="pause" size={14} />}
            action={
              <Button size="sm" variant="primary" onClick={onResume}>
                {t('members.resume')}
              </Button>
            }
          >
            {paused}
          </Callout>
        ) : m.partial ? (
          <Callout
            tone="quiet"
            action={
              <Button size="sm" loading={m.retryingPartial} onClick={m.retryPartial}>
                {t('members.retry')}
              </Button>
            }
          >
            {t('members.partial')}
          </Callout>
        ) : undefined)
      }
      figure={
        <FigurePicker
          workspace={workspace}
          value={m.draft.figure}
          onPick={(figure) => m.set({ figure })}
        />
      }
      sticker={
        reaction ? (
          <ReactionPicker
            workspace={workspace}
            value={reaction}
            onPick={pickReaction}
            trigger={
              <Sticker reaction={reaction} label={t('members.reaction')} onClick={() => {}} />
            }
          />
        ) : undefined
      }
      screen={<BrainPicker m={m} workspace={workspace} status={status} />}
      body={
        <SoulSlot
          value={m.draft.soul}
          onChange={(soul) => m.set({ soul })}
          label={t('members.soul')}
          placeholder={t('members.soulPlaceholder')}
          onFocusChange={onReading}
        />
      }
      deck={
        <>
          {!isNew && (
            // Only the Runtime pauses a member; the key lights up so it can be resumed.
            <ConsoleKey
              icon={paused ? 'play' : 'pause'}
              label={paused ? t('members.resume') : t('members.pause')}
              lit={!!paused}
              disabled={!paused}
              onClick={onResume}
              data-testid="member-pause"
            />
          )}
          {!isNew && (
            <ConsoleKey
              icon="trash"
              label={t('members.delete')}
              onClick={onDelete}
              data-testid="member-delete"
            />
          )}
          {!reaction && (
            <ReactionPicker
              workspace={workspace}
              value={null}
              onPick={pickReaction}
              trigger={<ConsoleKey icon="smile" label={t('members.reaction')} onClick={() => {}} />}
            />
          )}
          <span className={css.spacer} />
          <ConsoleKey
            go
            label={isNew ? t('members.create') : t('members.save')}
            lit={isNew ? canCreate : m.dirty}
            disabled={(isNew ? !canCreate : !m.dirty) || m.saving}
            onClick={onCommit}
            data-testid={isNew ? 'member-create' : 'member-save'}
          />
        </>
      }
      drawer={{ label: t('members.advanced'), content: <AdvancedDrawer m={m} /> }}
    />
  );
}
