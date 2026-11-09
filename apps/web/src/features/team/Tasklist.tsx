import { useRef, useState } from 'react';
import { Button, Callout, Icon, IconButton, InlineText, Menu, useToast } from '../../design';
import { ChecklistRow, Clipboard, VacantSlot } from '../../scene';
import { useT } from '../../lib/i18n';
import {
  useUpdateWorkspace,
  useRenameWorkspaceTasklist,
  useWorkspaceImage,
  useWorkspaceTasks,
  type WorkspaceTask,
} from '../../lib/queries/workspaces';
import type { Workspace, WorkspaceRef } from '../../lib/api/generated';
import './messages';

// The team's Feishu task list on a clipboard; a vacant outline while the task system is off.
// When the list can't be read the clipboard stays, with a retry and a way to turn it off.
export function TasklistBoard({
  workspace,
  onTurnedOff,
}: {
  workspace: Workspace;
  onTurnedOff: () => void;
}) {
  const t = useT();
  const toast = useToast();
  const failed = () => toast({ message: t('team.failed') });
  const request = { request_id: 'workspace_tasks', workspace: workspace.workspace };
  const tasks = useWorkspaceTasks(request);
  const rename = useRenameWorkspaceTasklist();
  const [draft, setDraft] = useState<string | null>(null);
  const cancelled = useRef(false);
  const commitName = () => {
    const name = draft?.trim();
    if (cancelled.current || !name || name === tasks.data?.name || rename.isPending) {
      cancelled.current = false;
      setDraft(null);
      return;
    }
    rename.mutate(
      { ...request, name },
      {
        onError: failed,
        onSettled: () => setDraft(null),
      },
    );
  };
  const { turn, turning } = useTurnTasks(workspace, onTurnedOff);
  const menu = (
    <Menu align="end" trigger={<IconButton icon="more" label={t('team.more')} size="sm" />}>
      <Menu.Item leading={<Icon name="ban" size={14} />} onSelect={() => turn(false)}>
        {t('team.turnOff')}
      </Menu.Item>
    </Menu>
  );
  // A list that has failed keeps its clipboard while it is read again, even with nothing loaded.
  if (tasks.isError || (!tasks.data && tasks.errorUpdateCount > 0))
    return (
      <Clipboard data-testid="tasklist-board" title={t('team.tasks')} actions={menu} footer={null}>
        <Callout
          tone="quiet"
          action={
            <Button size="sm" loading={tasks.isFetching} onClick={() => void tasks.refetch()}>
              {t('team.retry')}
            </Button>
          }
        >
          {t('team.unavailable')}
        </Callout>
      </Clipboard>
    );
  // Until the list has been read, the team's own setting decides what hangs on the wall, so a
  // slow Feishu answer never leaves the spot empty.
  const enabled = tasks.data ? tasks.data.enabled : configured(workspace);
  if (!tasks.data && enabled)
    return (
      <Clipboard data-testid="tasklist-board" title={t('team.tasks')} actions={menu} footer={null}>
        {null}
      </Clipboard>
    );
  if (!enabled)
    return (
      <VacantSlot
        data-testid="tasklist-board"
        shape="clipboard"
        label={t('team.tasks')}
        busy={turning}
        onClick={() => turn(true)}
      />
    );
  if (!tasks.data) return null;
  return (
    <Clipboard
      data-testid="tasklist-board"
      title={
        <InlineText
          mode="live"
          textStyle="title"
          aria-label={t('team.tasks')}
          value={draft ?? tasks.data.name ?? ''}
          placeholder={t('team.tasks')}
          aria-busy={rename.isPending}
          onChange={(value) => {
            if (!rename.isPending) setDraft(value);
          }}
          onBlur={commitName}
          onKeyDown={(e) => {
            if (e.nativeEvent.isComposing) return;
            if (e.key === 'Escape' || e.key === 'Enter') {
              e.preventDefault();
              cancelled.current = e.key === 'Escape';
              e.currentTarget.blur();
            }
          }}
        />
      }
      actions={menu}
      footer={null}
    >
      {tasks.data.items.map((task) => (
        <TaskRow key={task.id} task={task} workspace={workspace.workspace} />
      ))}
    </Clipboard>
  );
}

export function useTurnTasks(workspace: Workspace, onDone?: () => void) {
  const t = useT();
  const toast = useToast();
  const update = useUpdateWorkspace(`tasks-${workspace.workspace?.workspace_id}`);
  const turn = (enabled: boolean) => {
    const config: unknown = JSON.parse(workspace.provider_config_json || '{}');
    update.mutate(
      {
        request_id: 'update_workspace_tasks',
        workspace: workspace.workspace,
        name: null,
        rules: null,
        provider_config_json: JSON.stringify({
          ...(config && typeof config === 'object' ? config : {}),
          task_system_enabled: enabled,
        }),
        cover_resource_id: null,
        provider_secrets_json: null,
      },
      { onSuccess: onDone, onError: () => toast({ message: t('team.failed') }) },
    );
  };
  return { turn, turning: update.isPending };
}

export function configured(workspace: Workspace) {
  try {
    const config: unknown = JSON.parse(workspace.provider_config_json || '{}');
    return !!config && typeof config === 'object' && 'task_system_enabled' in config
      ? config.task_system_enabled === true
      : false;
  } catch {
    return false;
  }
}

function TaskRow({ task, workspace }: { task: WorkspaceTask; workspace: WorkspaceRef | null }) {
  const resource = task.assignee?.avatar;
  const image = useWorkspaceImage(
    { request_id: 'task_assignee', workspace, resource_id: resource?.resource_id ?? 0 },
    !!resource,
  );
  const done = task.completed;
  return (
    <ChecklistRow
      data-testid={`task-${task.id}`}
      done={done}
      text={task.text}
      avatar={
        image.data?.resource
          ? `data:${image.data.resource.media_type};base64,${image.data.bytes}`
          : undefined
      }
    />
  );
}
