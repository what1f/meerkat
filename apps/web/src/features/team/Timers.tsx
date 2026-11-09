import { useState } from 'react';
import { Avatar, Button, Callout, Closeup, Icon, Popover, useToast } from '../../design';
import { ScheduleCard, TimeCard, TimeCardRack, type HoleHandle } from '../../scene';
import { useLanguage, useT } from '../../lib/i18n';
import { dataUrl } from '../../lib/assets';
import { useAgentAvatar, useAgents } from '../../lib/queries/agents';
import { useCancelTimer, useRestoreTimer, useTimers } from '../../lib/queries/workspaces';
import type { Agent, Timer, Workspace } from '../../lib/api/generated';
import { clock, dayOf, fullDate, repeatOf } from './timerText';
import './messages';

// On the wall the timers are a small schedule of what comes next; a click opens the rack.
export function Timers({ workspace, handle }: { workspace: Workspace; handle?: HoleHandle }) {
  const t = useT();
  const language = useLanguage();
  const [open, setOpen] = useState(false);
  const timers = useTimers({ request_id: 'list_timers', workspace: workspace.workspace });
  const agents = useAgents({ request_id: 'timer_agents', workspace: workspace.workspace });
  const items = [...(timers.data ?? [])].sort((a, b) => a.next_fire_at - b.next_fire_at);
  return (
    <>
      <ScheduleCard
        ref={handle?.ref}
        style={handle?.style}
        data-testid="timers"
        title={t('team.timers')}
        alert={timers.isError}
        entries={items.slice(0, 4).map((timer) => ({
          day: dayOf(timer.next_fire_at, language),
          time: clock(timer.next_fire_at, language),
          avatar: <TimerAvatar timer={timer} agent={agentOf(agents.data?.agents, timer)} />,
        }))}
        onClick={() => setOpen(true)}
      />
      <Closeup open={open} onClose={() => setOpen(false)} label={t('team.timers')}>
        <TimerRack workspace={workspace} />
      </Closeup>
    </>
  );
}

const agentOf = (agents: Agent[] | undefined, timer: Timer) =>
  agents?.find((a) => a.agent?.agent_id === timer.agent?.agent_id);

function TimerAvatar({ timer, agent }: { timer: Timer; agent?: Agent }) {
  const avatar = useAgentAvatar(
    { request_id: 'timer_avatar', agent: timer.agent },
    !!agent?.avatar_path,
  );
  const file = avatar.data?.avatar;
  return <Avatar size={20} src={file ? dataUrl(file) : undefined} />;
}

// The timers members set to wake themselves, one card each in a rack.
function TimerRack({ workspace }: { workspace: Workspace }) {
  const t = useT();
  const request = { request_id: 'list_timers', workspace: workspace.workspace };
  const timers = useTimers(request);
  const agents = useAgents({ request_id: 'timer_agents', workspace: workspace.workspace });
  if (timers.isError)
    return (
      <TimeCardRack data-testid="timer-rack" label={t('team.timers')}>
        {[
          <Callout
            key="error"
            tone="quiet"
            action={
              <Button size="sm" loading={timers.isFetching} onClick={() => void timers.refetch()}>
                {t('team.retry')}
              </Button>
            }
          >
            {t('team.timersUnavailable')}
          </Callout>,
        ]}
      </TimeCardRack>
    );
  const items = [...(timers.data ?? [])].sort((a, b) => a.next_fire_at - b.next_fire_at);
  return (
    <TimeCardRack data-testid="timer-rack" label={t('team.timers')}>
      {items.map((timer) => (
        <TimerCard
          key={timer.timer_id}
          timer={timer}
          workspace={workspace}
          agent={agentOf(agents.data?.agents, timer)}
        />
      ))}
    </TimeCardRack>
  );
}

function TimerCard({
  timer,
  workspace,
  agent,
}: {
  timer: Timer;
  workspace: Workspace;
  agent?: Agent;
}) {
  const t = useT();
  const language = useLanguage();
  const toast = useToast();
  const [open, setOpen] = useState(false);
  const cancel = useCancelTimer();
  const restore = useRestoreTimer();
  const id = timer.agent?.agent_id ?? '';
  const avatar = useAgentAvatar(
    { request_id: 'timer_avatar', agent: timer.agent },
    !!agent?.avatar_path,
  );
  const file = avatar.data?.avatar;
  const src = file ? dataUrl(file) : undefined;
  const repeat = repeatOf(timer, language);
  const repeats = timer.repeat.kind !== 'once';
  const request = { request_id: 'timer', workspace: workspace.workspace, timer_id: timer.timer_id };
  const remove = () =>
    cancel.mutate(request, {
      onSuccess: () => {
        setOpen(false);
        toast({
          message: t('team.timerRemoved'),
          action: {
            label: t('team.undo'),
            onAction: () =>
              restore.mutate(request, { onError: () => toast({ message: t('team.failed') }) }),
          },
        });
      },
      onError: () => toast({ message: t('team.failed') }),
    });
  return (
    <Popover
      align="center"
      open={open}
      onOpenChange={setOpen}
      trigger={
        <TimeCard
          data-testid={`timer-${timer.timer_id}`}
          avatar={<Avatar size={20} src={src} />}
          when={repeats ? (repeat ?? t('team.repeats')) : dayOf(timer.next_fire_at, language)}
          time={clock(timer.next_fire_at, language)}
          repeats={repeats}
          text={timer.prompt}
          lifted={open}
        />
      }
    >
      <TimeCard.Note
        avatar={<Avatar size={26} src={src} />}
        who={agent?.name ?? id}
        text={timer.prompt}
        facts={[
          { icon: 'clock', text: fullDate(timer.next_fire_at, language) },
          ...(repeats
            ? [
                {
                  icon: 'repeat' as const,
                  text:
                    repeat ??
                    (timer.repeat.kind === 'cron' && <code>{timer.repeat.expression}</code>),
                },
              ]
            : []),
          ...(timer.chat_name ? [{ icon: 'msg' as const, text: timer.chat_name }] : []),
        ]}
        action={
          <Button
            size="sm"
            variant="quiet"
            leading={<Icon name="trash" size={14} />}
            loading={cancel.isPending}
            onClick={remove}
          >
            {t('team.removeTimer')}
          </Button>
        }
      />
    </Popover>
  );
}
