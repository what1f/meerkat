import { useRef, useState } from 'react';
import { Badge, BadgeVerso, FlipCard, Bench, Board, Hang } from '../../scene';
import { BrandMark, EmptyState, InlineText, useToast } from '../../design';
import { useT } from '../../lib/i18n';
import { avatarUrl, resourceUrl } from '../../lib/assets';
import type { HumanTeamMember } from '../../lib/api/generated';
import {
  useHumanMembers,
  useSyncHumanProfile,
  useUpdateHumanPosition,
} from '../../lib/queries/workspaces';
import { defaultAvatar } from './avatars';
import type { MembersPageProps } from './MembersPage';

export function HumanCard({
  workspace,
  person,
  provider,
  onOpen,
}: {
  workspace: string;
  person: HumanTeamMember;
  provider: 'feishu' | 'telegram';
  onOpen: () => void;
}) {
  const t = useT();
  const toast = useToast();
  const sync = useSyncHumanProfile();
  const [flipped, setFlipped] = useState(false);
  const actions = [
    {
      label: t('members.sync'),
      busy: sync.isPending,
      onClick: () => {
        if (sync.isPending) return;
        sync.mutate(
          {
            request_id: crypto.randomUUID(),
            workspace: { workspace_id: workspace },
            person_id: person.person_id,
          },
          {
            onSuccess: () => setFlipped(false),
            onError: () => toast({ message: t('members.failed') }),
          },
        );
      },
    },
    { label: t('members.editPosition'), onClick: onOpen },
  ];
  const fallback = avatarUrl(defaultAvatar(person.person_id));
  return (
    <Hang lanyard="lead" swingOnHover data-testid={`human-card-${person.person_id}`}>
      <FlipCard
        flipped={flipped}
        onFlip={() => setFlipped(!flipped)}
        back={<BadgeVerso mark={<BrandMark brand={provider} />} actions={actions} />}
        front={
          <Badge
            size="card"
            avatar={{
              src: person.avatar ? resourceUrl(person.avatar.resource_id, workspace) : fallback,
              fallback,
            }}
            name={person.name || t('members.human')}
            role={person.position}
            bio={person.signature || undefined}
          />
        }
      />
    </Hang>
  );
}

export function HumanBench({
  workspace,
  id,
  onBack,
  frame,
}: {
  workspace: string;
  id: string;
  onBack: () => void;
  frame: MembersPageProps['frame'];
}) {
  const t = useT();
  const humans = useHumanMembers({ request_id: 'humans', workspace: { workspace_id: workspace } });
  const person = humans.data?.find((p) => p.person_id === id);
  return frame(
    { back: { label: t('common.members'), onClick: onBack } },
    <Board layout="open">
      <Bench>
        {person ? (
          <HumanProfile key={id} workspace={workspace} person={person} />
        ) : humans.isPending ? (
          <Hang lanyard="lead">
            <Badge size="stage" avatar={{}} name={null} ghost />
          </Hang>
        ) : (
          <EmptyState title={t(humans.isError ? 'members.unavailable' : 'members.notFound')} />
        )}
      </Bench>
    </Board>,
    'open',
  );
}

function HumanProfile({ workspace, person }: { workspace: string; person: HumanTeamMember }) {
  const t = useT();
  const toast = useToast();
  const update = useUpdateHumanPosition();
  const [position, setPosition] = useState(person.position);
  const cancelled = useRef(false);
  const draft = useRef(person.position);
  const confirmed = useRef(person.position);
  const revision = useRef(0);
  const writes = useRef(Promise.resolve());
  const fallback = avatarUrl(defaultAvatar(person.person_id));
  const change = (value: string) => {
    draft.current = value;
    revision.current += 1;
    setPosition(value);
  };
  const save = () => {
    if (cancelled.current) {
      cancelled.current = false;
      return;
    }
    const value = draft.current;
    const edit = revision.current;
    // A blur during an earlier save is still a write. Its response may only
    // replace the draft if the user has made no newer edit.
    writes.current = writes.current.then(async () => {
      if (value === confirmed.current) return;
      try {
        const people = await update.mutateAsync({
          request_id: crypto.randomUUID(),
          workspace: { workspace_id: workspace },
          person_id: person.person_id,
          position: value,
        });
        confirmed.current = people.find((p) => p.person_id === person.person_id)?.position ?? value;
        if (revision.current === edit) {
          draft.current = confirmed.current;
          setPosition(confirmed.current);
        }
      } catch {
        if (revision.current === edit) {
          draft.current = confirmed.current;
          setPosition(confirmed.current);
        }
        toast({ message: t('members.failed') });
      }
    });
  };
  return (
    <Hang lanyard="lead" data-testid="human-profile">
      <Badge
        size="stage"
        avatar={{
          src: person.avatar ? resourceUrl(person.avatar.resource_id, workspace) : fallback,
          fallback,
        }}
        name={person.name || t('members.human')}
        bio={person.signature || undefined}
        role={
          <InlineText
            mode="live"
            value={position}
            onChange={change}
            placeholder={t('members.role')}
            textStyle="small"
            data-testid="human-position"
            aria-label={t('members.role')}
            onBlur={() => void save()}
            onKeyDown={(e) => {
              if (e.nativeEvent.isComposing) return;
              if (e.key === 'Escape') {
                e.preventDefault();
                cancelled.current = true;
                change(confirmed.current);
                e.currentTarget.blur();
              }
              if (e.key === 'Enter') {
                e.preventDefault();
                e.currentTarget.blur();
              }
            }}
          />
        }
      />
    </Hang>
  );
}
