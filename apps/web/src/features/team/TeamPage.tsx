import { useState, type ReactNode } from 'react';
import { useToast } from '../../design';
import { Board, PaperLabel, useHoleDrag, type HolePosition } from '../../scene';
import { useT } from '../../lib/i18n';
import { readStorage, writeStorage } from '../../lib/storage';
import { useUpdateWorkspace, useWorkspace } from '../../lib/queries/workspaces';
import type { Workspace } from '../../lib/api/generated';
import { RulesTag } from './RulesTag';
import { Tasklist } from './Tasklist';
import { ShiftToggle } from './ShiftToggle';
import './messages';

type Spot = 'rules' | 'tasks' | 'shift';
type Wall = Partial<Record<Spot, HolePosition>>;
// Hole pitch of the board.
const cell = 24;
// Spots as holes from the top left of the arrangement, and its width in holes. A board too
// narrow for one row hangs the shift sign under the rules.
const layouts: { span: number; spots: Record<Spot, HolePosition> }[] = [
  { span: 39, spots: { rules: { x: 7, y: 4 }, tasks: { x: 24, y: 4 }, shift: { x: 36, y: 4 } } },
  { span: 30, spots: { rules: { x: 7, y: 4 }, tasks: { x: 22, y: 4 }, shift: { x: 7, y: 24 } } },
];
const plain = {
  span: 22,
  spots: { rules: { x: 7, y: 4 }, tasks: { x: 7, y: 4 }, shift: { x: 19, y: 4 } },
};
const isHole = (v: unknown): v is HolePosition =>
  !!v &&
  typeof v === 'object' &&
  Number.isInteger((v as HolePosition).x) &&
  Number.isInteger((v as HolePosition).y);
const isWall = (v: unknown): v is Wall =>
  !!v && typeof v === 'object' && Object.values(v).every(isHole);

export type TeamPageProps = {
  workspaceId: string;
  frame: (bar: { title: string }, body: ReactNode) => ReactNode;
};

export function TeamPage({ workspaceId, frame }: TeamPageProps) {
  const t = useT();
  const { data } = useWorkspace(
    { request_id: 'get_workspace', workspace: { workspace_id: workspaceId } },
    !!workspaceId,
  );
  return frame(
    { title: t('common.team') },
    !data || data.workspace?.workspace_id !== workspaceId ? (
      <Board layout="free">{null}</Board>
    ) : (
      <TeamWall key={workspaceId} workspace={data} />
    ),
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
  const feishu = workspace.provider_id === 'feishu';
  const layout = !feishu
    ? plain
    : (layouts.find((l) => l.span <= (cols ?? 0)) ?? layouts[layouts.length - 1]!);
  const spots: Spot[] = feishu ? ['rules', 'tasks', 'shift'] : ['rules', 'shift'];
  const left = Math.max(0, Math.floor(((cols ?? 0) - layout.span) / 2));
  const items =
    cols === null
      ? []
      : spots.map((key) => ({
          key,
          kind: key,
          pos: saved[key] ?? { x: left + layout.spots[key].x, y: layout.spots[key].y },
        }));
  const handles = useHoleDrag(items, (key, pos) => {
    const next = Object.fromEntries(
      items.map((item) => [item.key, item.key === key ? pos : item.pos]),
    );
    setSaved(next);
    writeStorage(storageKey, next);
  });
  const content: Record<Spot, ReactNode> = {
    rules: <RulesTag workspace={workspace} />,
    tasks: <Tasklist workspace={workspace} />,
    shift: <ShiftToggle workspace={workspace} />,
  };
  return (
    <Board layout="free">
      <div
        style={{ left: '50%', top: 0 }}
        ref={(el) => {
          if (el?.parentElement && cols === null)
            setCols(Math.floor(el.parentElement.clientWidth / cell));
        }}
      >
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
      </div>
      {items.map(({ key }) => {
        const handle = handles[key];
        return (
          <div
            key={key}
            ref={handle?.ref}
            style={handle?.style}
            data-dragging={handle?.dragging || undefined}
          >
            {content[key]}
          </div>
        );
      })}
    </Board>
  );
}
