// Natural frame sizes. Every desk pose shares one canvas; standing and carried frames vary by figure.
const desk = { width: 169, height: 250 };
const deskPoses = ['work', 'sleep'] as const;
const own: Record<string, Record<'carry' | 'stand', [number, number]>> = {
  '01': { carry: [96, 180], stand: [120, 180] },
  '04': { carry: [144, 203], stand: [129, 202] },
  '07': { carry: [99, 184], stand: [117, 190] },
  '11': { carry: [98, 184], stand: [125, 185] },
  '14': { carry: [111, 179], stand: [129, 190] },
  '15': { carry: [137, 168], stand: [109, 170] },
};
export const figures: Record<
  string,
  Record<string, { width: number; height: number }>
> = Object.fromEntries(
  Object.entries(own).map(([id, { carry, stand }]) => [
    id,
    {
      ...Object.fromEntries(deskPoses.map((pose) => [pose, desk])),
      carry: { width: carry[0], height: carry[1] },
      stand: { width: stand[0], height: stand[1] },
    },
  ]),
);
const frames = import.meta.glob<string>('../assets/figures/*/*.png', {
  eager: true,
  query: '?url',
  import: 'default',
});
export type Pose = 'work' | 'sleep' | 'carry' | 'stand';
/** What a built-in figure does at its desk; each move is a strip of eight frames. */
export type Move = 'work' | 'sip' | 'phone' | 'sleep' | 'stretch' | 'fret' | 'cheer' | 'think';
export const deskMoves: readonly Move[] = [
  'work',
  'sip',
  'phone',
  'sleep',
  'stretch',
  'fret',
  'cheer',
  'think',
];
const strips = import.meta.glob<string>('../assets/figures/*/desk/*.webp', {
  eager: true,
  query: '?url',
  import: 'default',
});
/** An imported figure pack: a frame URL per pose it has, sized by `scale` from its natural pixels. */
export type FigurePack = { frames: Partial<Record<Pose, string>>; scale: number };
export type FigureFrame = { src: string; width?: number; height?: number; factor: number };
const builtins = Object.keys(figures).sort();
// The same seed always gets the same built-in figure.
const pick = (seed: string) => {
  let h = 0;
  for (const c of seed) h = (h * 31 + c.charCodeAt(0)) >>> 0;
  return builtins[h % builtins.length] ?? '';
};
/** The built-in figure that sits at a desk, or null when a pack brings its own working frame. */
export function deskFigure(id: string, pack?: FigurePack, seed = id): string | null {
  if (pack?.frames.work) return null;
  return !pack && figures[id] ? id : pick(seed);
}
export const moveStrip = (figure: string, move: Move) =>
  strips[`../assets/figures/${figure}/desk/${move}.webp`];
// Built-in frames have known sizes; a pack frame is sized once it has loaded.
// `default`, an unknown id or a pack without this pose falls back to a built-in figure chosen by `seed`.
export function figureFrame(
  id: string,
  pose: Pose,
  scale = 1,
  pack?: FigurePack,
  seed = id,
): FigureFrame | null {
  const packed = pack?.frames[pose];
  if (pack && packed) return { src: packed, factor: pack.scale * scale };
  const builtin = !pack && figures[id] ? id : pick(seed);
  const size = figures[builtin]?.[pose];
  const src = frames[`../assets/figures/${builtin}/${pose}.png`];
  return size && src
    ? { src, width: size.width * scale, height: size.height * scale, factor: scale }
    : null;
}
