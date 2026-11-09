import { Fragment, useCallback, useState, type ReactNode, type RefCallback } from 'react';
import { useToast } from '../../design';
import { Board, PaperLabel, holePitch, useHoleDrag, type HoleHandle } from '../../scene';
import { useT } from '../../lib/i18n';
import { readStorage, writeStorage } from '../../lib/storage';
import { useUpdateWorkspace, useWorkspace } from '../../lib/queries/workspaces';
import type { Workspace } from '../../lib/api/generated';
import {
  bareLayouts,
  isWall,
  layouts,
  plain,
  type Size,
  type Spot,
  type Wall,
  hang,
} from './wallLayout';
import { RulesTag } from './RulesTag';
import { TaskPad, useTasksOn } from './TaskPad';
import { ShiftToggle } from './ShiftToggle';
import { Timers } from './Timers';
import './messages';

export type TeamPageProps = {
  workspaceId: string;
  frame: (bar: Record<string, never>, body: ReactNode, board?: 'wall' | 'open') => ReactNode;
};

export function TeamPage({ workspaceId, frame }: TeamPageProps) {
  const { data } = useWorkspace(
    { request_id: 'get_workspace', workspace: { workspace_id: workspaceId } },
    !!workspaceId,
  );
  return frame(
    {},
    !data || data.workspace?.workspace_id !== workspaceId ? (
      <Board layout="free">{null}</Board>
    ) : (
      <TeamWall key={workspaceId} workspace={data} />
    ),
    'wall',
  );
}

function TeamWall({ workspace }: { workspace: Workspace }) {
  const t = useT();
  const toast = useToast();
  const rename = useUpdateWorkspace();
  const storageKey = `team.wall.${workspace.workspace?.workspace_id}`;
  const [saved, setSaved] = useState(() => readStorage<Wall>(storageKey, {}, isWall));
  // Default spots are centred on the board, so they wait for its width.
  const [cols, setCols] = useState<number | null>(null);
  // The board's width in holes follows the window.
  const measure = useCallback((el: HTMLDivElement | null) => {
    const board = el?.parentElement;
    if (!board) return;
    const update = () => setCols(Math.floor(board.clientWidth / holePitch));
    update();
    const observer = new ResizeObserver(update);
    observer.observe(board);
    return () => observer.disconnect();
  }, []);
  const feishu = workspace.provider_id === 'feishu';
  const tasksOn = useTasksOn(workspace, feishu);
  const tiers = tasksOn ? layouts : bareLayouts;
  const layout = !feishu
    ? plain
    : (tiers.find((l) => l.span <= (cols ?? 0)) ?? tiers[tiers.length - 1]!);
  const spots: Spot[] = feishu
    ? ['rules', 'tasks', 'timers', 'shift']
    : ['rules', 'timers', 'shift'];
  // The size of each hung piece follows its content.
  const [sizes, setSizes] = useState<Partial<Record<Spot, Size>>>({});
  const items = cols === null ? [] : hang(spots, saved, layout, cols, sizes);
  const handles = useHoleDrag(items, (key, pos) => {
    const next = { ...saved, [key]: pos };
    setSaved(next);
    writeStorage(storageKey, next);
  });
  // Measure each piece's outer element, the one the drag moves.
  const measured = (key: Spot): HoleHandle | undefined => {
    const handle = handles[key];
    if (!handle) return undefined;
    const ref: RefCallback<HTMLElement> = (el) => {
      const release = handle.ref(el);
      if (!el) return release;
      el.dataset.spot = key;
      const read = () =>
        setSizes((all) =>
          all[key]?.w === el.offsetWidth && all[key].h === el.offsetHeight
            ? all
            : { ...all, [key]: { w: el.offsetWidth, h: el.offsetHeight } },
        );
      read();
      const observer = new ResizeObserver(read);
      observer.observe(el);
      return () => {
        observer.disconnect();
        if (typeof release === 'function') release();
      };
    };
    return { ...handle, ref };
  };
  const content: Record<Spot, (handle: HoleHandle | undefined) => ReactNode> = {
    rules: (handle) => <RulesTag workspace={workspace} handle={handle} />,
    tasks: (handle) => <TaskPad workspace={workspace} handle={handle} />,
    timers: (handle) => <Timers workspace={workspace} handle={handle} />,
    shift: (handle) => <ShiftToggle workspace={workspace} handle={handle} />,
  };
  return (
    <Board
      layout="free"
      lead={
        <PaperLabel
          data-testid="team-name"
          value={workspace.name}
          label={t('team.name')}
          onCommit={(name) => {
            if (!name.trim() || name.trim() === workspace.name) return;
            rename.mutate(
              {
                request_id: 'update_workspace_name',
                workspace: workspace.workspace,
                name: name.trim(),
                rules: null,
                provider_config_json: null,
                cover_resource_id: null,
                provider_secrets_json: null,
              },
              { onError: () => toast({ message: t('team.failed') }) },
            );
          }}
        />
      }
    >
      {/* Default spots are centred on the board, so they wait for its width. */}
      <div aria-hidden="true" ref={measure} />
      {items.map(({ key }) => (
        <Fragment key={key}>{content[key](measured(key))}</Fragment>
      ))}
    </Board>
  );
}
