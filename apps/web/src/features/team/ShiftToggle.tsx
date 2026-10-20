import { useToast } from '../../design';
import { ShiftSign, type HoleHandle } from '../../scene';
import { useT } from '../../lib/i18n';
import { useSetWorkspaceState } from '../../lib/queries/workspaces';
import type { Workspace } from '../../lib/api/generated';
import './messages';

// Off shift the whole team stops taking messages.
export function ShiftToggle({ workspace, handle }: { workspace: Workspace; handle?: HoleHandle }) {
  const t = useT();
  const toast = useToast();
  // Toggles run in order, so the last one chosen is the one that stays.
  const shift = useSetWorkspaceState(`shift-${workspace.workspace?.workspace_id}`);
  const desired = shift.isPending ? shift.variables.desired_state : workspace.desired_state;
  const on = desired !== 'paused';
  return (
    <ShiftSign
      ref={handle?.ref}
      style={handle?.style}
      data-testid="team-shift"
      on={on}
      onToggle={() =>
        shift.mutate(
          {
            request_id: 'set_workspace_state',
            workspace: workspace.workspace,
            desired_state: on ? 'paused' : 'running',
          },
          { onError: () => toast({ message: t('team.failed') }) },
        )
      }
      onLabel={t('team.on')}
      offLabel={t('team.off')}
      label={t('team.shift')}
    />
  );
}
