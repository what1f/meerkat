import { Button, Callout, Icon, IconButton, Menu, useToast } from '../../design';
import { ChecklistRow, Clipboard, VacantSlot } from '../../scene';
import { useT } from '../../lib/i18n';
import {
  useCreateWorkspaceTask,
  useSetWorkspaceTaskCompleted,
  useUpdateWorkspace,
  useWorkspaceImage,
  useWorkspaceTasks,
  type WorkspaceTask,
} from '../../lib/queries/workspaces';
import type { Workspace, WorkspaceRef } from '../../lib/api/generated';
import './messages';

// The team's Feishu task list on a clipboard; a vacant outline while the task system is off.
// When the list can't be read the clipboard stays, with a retry and a way to turn it off.
export function Tasklist({ workspace }: { workspace: Workspace }) {
  const t = useT();
  const toast = useToast();
  const failed = () => toast({ message: t('team.failed') });
  const request = { request_id: 'workspace_tasks', workspace: workspace.workspace };
  const tasks = useWorkspaceTasks(request);
  const update = useUpdateWorkspace(`tasks-${workspace.workspace?.workspace_id}`);
  const create = useCreateWorkspaceTask();
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
      { onError: failed },
    );
  };
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
      <Clipboard data-testid="tasklist" title={t('team.tasks')} actions={menu} footer={null}>
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
  if (!tasks.data) return null;
  if (!tasks.data.enabled)
    return (
      <VacantSlot
        data-testid="tasklist"
        shape="clipboard"
        label={t('team.tasks')}
        onClick={() => turn(true)}
      />
    );
  return (
    <Clipboard
      data-testid="tasklist"
      title={tasks.data.name || t('team.tasks')}
      actions={menu}
      footer={
        <ChecklistRow.New
          placeholder={t('team.newTask')}
          onAdd={(text) => create.mutate({ ...request, text }, { onError: failed })}
        />
      }
    >
      {tasks.data.items.map((task) => (
        <TaskRow key={task.id} task={task} workspace={workspace.workspace} onError={failed} />
      ))}
    </Clipboard>
  );
}

function TaskRow({
  task,
  workspace,
  onError,
}: {
  task: WorkspaceTask;
  workspace: WorkspaceRef | null;
  onError: () => void;
}) {
  const complete = useSetWorkspaceTaskCompleted();
  const resource = task.assignee?.avatar;
  const image = useWorkspaceImage(
    { request_id: 'task_assignee', workspace, resource_id: resource?.resource_id ?? 0 },
    !!resource,
  );
  const done = complete.isPending ? complete.variables.completed : task.completed;
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
      onToggle={() =>
        complete.mutate(
          { request_id: 'set_task_completed', workspace, id: task.id, completed: !done },
          { onError },
        )
      }
    />
  );
}
