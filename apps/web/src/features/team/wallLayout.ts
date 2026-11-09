import { holePitch, type HolePosition } from '../../scene';

const cell = holePitch;
export type Spot = 'rules' | 'tasks' | 'timers' | 'shift';
export type Wall = Partial<Record<Spot, HolePosition>>;
// Spots as holes from the top left of the arrangement, and its width in holes. The task pad and
// the timer schedule hang one over the other beside the rules; a narrow board moves the shift sign
// under them.
export const layouts: { span: number; spots: Record<Spot, HolePosition> }[] = [
  {
    span: 31,
    spots: {
      rules: { x: 7, y: 1 },
      tasks: { x: 19, y: 1 },
      timers: { x: 19, y: 11 },
      shift: { x: 28, y: 1 },
    },
  },
  {
    span: 24,
    spots: {
      rules: { x: 7, y: 1 },
      tasks: { x: 19, y: 1 },
      timers: { x: 19, y: 11 },
      shift: { x: 19, y: 21 },
    },
  },
];
// With the task list off its slot is only an outline, so the rules take the middle of the board
// and the shift sign, the schedule and the slot hang beside them.
export const bareLayouts: typeof layouts = [
  {
    span: 48,
    spots: {
      rules: { x: 24, y: 1 },
      tasks: { x: 10, y: 1 },
      shift: { x: 38, y: 1 },
      timers: { x: 38, y: 7 },
    },
  },
  {
    span: 36,
    spots: {
      rules: { x: 18, y: 1 },
      tasks: { x: 18, y: 21 },
      shift: { x: 30, y: 1 },
      timers: { x: 30, y: 7 },
    },
  },
  {
    span: 24,
    spots: {
      rules: { x: 12, y: 1 },
      tasks: { x: 12, y: 21 },
      shift: { x: 22, y: 1 },
      timers: { x: 12, y: 31 },
    },
  },
];
// Holes between a spot's peg and its edge: whatever the saved position, it stays on the board.
export const reach: Record<Spot, number> = { rules: 7, tasks: 4, timers: 4, shift: 2 };
// What a spot takes up on the wall, the whole hung piece: its string, peg and paper. Until
// the piece has been measured this stands in for it.
export type Size = { w: number; h: number };
const guess: Record<Spot, Size> = {
  rules: { w: 352, h: 466 },
  tasks: { w: 184, h: 220 },
  timers: { w: 184, h: 220 },
  shift: { w: 120, h: 112 },
};
const rectOf = (pos: HolePosition, size: Size) => ({
  left: pos.x * cell + cell / 2 - size.w / 2,
  top: pos.y * cell,
  size,
});
const overlaps = (a: ReturnType<typeof rectOf>, b: ReturnType<typeof rectOf>) =>
  a.left < b.left + b.size.w &&
  b.left < a.left + a.size.w &&
  a.top < b.top + b.size.h &&
  b.top < a.top + a.size.h;
// A team without a task list: the rules in the middle, the sign and the schedule beside them.
export const plain = {
  span: 36,
  spots: {
    rules: { x: 18, y: 1 },
    tasks: { x: 18, y: 21 },
    shift: { x: 30, y: 1 },
    timers: { x: 30, y: 7 },
  },
};
const isHole = (v: unknown): v is HolePosition =>
  !!v &&
  typeof v === 'object' &&
  Number.isInteger((v as HolePosition).x) &&
  Number.isInteger((v as HolePosition).y);
export const isWall = (v: unknown): v is Wall =>
  !!v && typeof v === 'object' && Object.values(v).every(isHole);

type Layout = (typeof layouts)[number];
// Where each spot hangs on a board `cols` holes wide. Whenever a spot would cover another (a narrow
// board, a piece grown taller), it goes back to the layout for this width, or under everything
// already hung. The saved wall itself is never changed.
export function hang(
  spots: Spot[],
  saved: Wall,
  layout: Layout,
  cols: number,
  sizes: Partial<Record<Spot, Size>>,
) {
  const sizeOf = (key: Spot) => sizes[key] ?? guess[key];
  const left = Math.max(0, Math.floor((cols - layout.span) / 2));
  const onBoard = (pos: HolePosition, key: Spot) => ({
    ...pos,
    x: Math.max(reach[key], Math.min(pos.x, cols - 1 - reach[key])),
  });
  const items: { key: Spot; kind: Spot; pos: HolePosition }[] = [];
  for (const key of spots) {
    const fallback = { x: left + layout.spots[key].x, y: layout.spots[key].y };
    const clash = (pos: HolePosition) =>
      items.some((item) => overlaps(rectOf(pos, sizeOf(key)), rectOf(item.pos, sizeOf(item.key))));
    let pos = onBoard(saved[key] ?? fallback, key);
    if (clash(pos)) pos = onBoard(fallback, key);
    if (clash(pos))
      pos = {
        ...pos,
        y: Math.ceil(
          Math.max(...items.map((item) => item.pos.y * cell + sizeOf(item.key).h)) / cell,
        ),
      };
    items.push({ key, kind: key, pos });
  }
  return items;
}
