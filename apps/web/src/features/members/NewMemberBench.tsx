import { useMemberDraft } from './useMemberDraft';
import { MemberStage } from './MemberStage';

// A member made away from the members page: they open as soon as they have joined.
export function NewMemberBench({
  workspace,
  channel,
  onJoined,
}: {
  workspace: string;
  channel: 'feishu' | 'telegram';
  onJoined: (id: string) => void;
}) {
  const m = useMemberDraft(workspace, null, channel);
  return (
    <MemberStage
      m={m}
      workspace={workspace}
      id={null}
      channel={channel}
      autoOpen
      onCreated={onJoined}
      onResume={() => undefined}
      onDelete={() => undefined}
    />
  );
}
