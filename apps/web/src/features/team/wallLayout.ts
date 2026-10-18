import { holePitch, type HolePosition } from '../../scene';

const cell = holePitch;
export type Spot = 'rules' | 'tasks' | 'shift';
export type Wall = Partial<Record<Spot, HolePosition>>;
// Spots as holes from the top left of the arrangement, and its width in holes. A board too
// narrow for one row hangs the shift sign under the rules, and a narrower one the list under them.
export const layouts: { span: number; spots: Record<Spot, HolePosition> }[] = [
  { span: 36, spots: { rules: { x: 7, y: 1 }, tasks: { x: 24, y: 1 }, shift: { x: 34, y: 1 } } },
  { span: 30, spots: { rules: { x: 7, y: 1 }, tasks: { x: 22, y: 1 }, shift: { x: 7, y: 21 } } },
  { span: 24, spots: { rules: { x: 7, y: 1 }, tasks: { x: 7, y: 21 }, shift: { x: 17, y: 1 } } },
];
// Holes between a spot's peg and its edge: whatever the saved position, it stays on the board.
export const reach: Record<Spot, number> = { rules: 7, tasks: 7, shift: 2 };
// What a spot takes up on the wall, the whole hung piece: its string, peg and paper. Until
// the piece has been measured this stands in for it.
export type Size = { w: number; h: number };
const guess: Record<Spot, Size> = {
  rules: { w: 352, h: 466 },
  tasks: { w: 352, h: 476 },
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
export const plain = {
  span: 36,
  spots: { rules: { x: 7, y: 1 }, tasks: { x: 7, y: 1 }, shift: { x: 17, y: 1 } },
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
