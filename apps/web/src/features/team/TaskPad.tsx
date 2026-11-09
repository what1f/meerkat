import { useState } from 'react';
import { Closeup } from '../../design';
import { MemoPad, VacantSlot, type HoleHandle } from '../../scene';
import { useT } from '../../lib/i18n';
import { useWorkspaceTasks } from '../../lib/queries/workspaces';
import type { Workspace } from '../../lib/api/generated';
import { TasklistBoard, configured, useTurnTasks } from './Tasklist';
import './messages';

// On the wall the list is a small memo pad; a click opens the clipboard. While the task system
// is off the pad is a vacant outline that turns it on.
export function TaskPad({ workspace, handle }: { workspace: Workspace; handle?: HoleHandle }) {
  const t = useT();
  const [open, setOpen] = useState(false);
  const tasks = useWorkspaceTasks({
    request_id: 'workspace_tasks',
    workspace: workspace.workspace,
  });
  const { turn, turning } = useTurnTasks(workspace);
  const failing = tasks.isError || (!tasks.data && tasks.errorUpdateCount > 0);
  const enabled = failing || (tasks.data ? tasks.data.enabled : configured(workspace));
  const peg = { ref: handle?.ref, style: handle?.style };
  if (!enabled && !open)
    return (
      <VacantSlot
        {...peg}
        data-testid="tasklist"
        shape="pad"
        label={t('team.tasks')}
        busy={turning}
        onClick={() => turn(true)}
      />
    );
  const name = tasks.data?.name || t('team.tasks');
  return (
    <>
      <MemoPad
        {...peg}
        data-testid="tasklist"
        title={name}
        items={(tasks.data?.items ?? []).filter((task) => !task.completed).map((task) => task.text)}
        alert={failing}
        onClick={() => setOpen(true)}
      />
      <Closeup open={open} onClose={() => setOpen(false)} label={name}>
        <TasklistBoard workspace={workspace} onTurnedOff={() => setOpen(false)} />
      </Closeup>
    </>
  );
}

// Whether the task list hangs as a pad rather than an empty slot: what Feishu said, else the setting.
export function useTasksOn(workspace: Workspace, feishu: boolean) {
  const tasks = useWorkspaceTasks(
    { request_id: 'workspace_tasks', workspace: workspace.workspace },
    feishu,
  );
  if (tasks.isError || (!tasks.data && tasks.errorUpdateCount > 0)) return true;
  return tasks.data ? tasks.data.enabled : configured(workspace);
}
