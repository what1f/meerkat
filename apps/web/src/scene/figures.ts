export const figures: Record<string, Record<string, { width: number; height: number }>> = {
  '01': {
    carry: {
      width: 96,
      height: 180,
    },
    sleep: {
      width: 169,
      height: 250,
    },
    stand: {
      width: 120,
      height: 180,
    },
    work: {
      width: 169,
      height: 250,
    },
  },
  '04': {
    carry: {
      width: 144,
      height: 203,
    },
    sleep: {
      width: 169,
      height: 250,
    },
    stand: {
      width: 129,
      height: 202,
    },
    work: {
      width: 169,
      height: 250,
    },
  },
  '07': {
    carry: {
      width: 99,
      height: 184,
    },
    sleep: {
      width: 169,
      height: 250,
    },
    stand: {
      width: 117,
      height: 190,
    },
    work: {
      width: 169,
      height: 250,
    },
  },
  '11': {
    carry: {
      width: 98,
      height: 184,
    },
    sleep: {
      width: 169,
      height: 250,
    },
    stand: {
      width: 125,
      height: 185,
    },
    work: {
      width: 169,
      height: 250,
    },
  },
  '14': {
    carry: {
      width: 111,
      height: 179,
    },
    sleep: {
      width: 169,
      height: 250,
    },
    stand: {
      width: 129,
      height: 190,
    },
    work: {
      width: 169,
      height: 250,
    },
  },
  '15': {
    carry: {
      width: 137,
      height: 168,
    },
    sleep: {
      width: 169,
      height: 250,
    },
    stand: {
      width: 109,
      height: 170,
    },
    work: {
      width: 169,
      height: 250,
    },
  },
};
const frames = import.meta.glob<string>('../assets/figures/*/*.png', {
  eager: true,
  query: '?url',
  import: 'default',
});
export type Pose = 'work' | 'sleep' | 'carry' | 'stand';
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
