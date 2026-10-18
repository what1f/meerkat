import { useState } from 'react';
import type { UIProps } from '../design/index';
import { figureFrame, type FigurePack, type Pose } from './figures';
import css from './Figure.module.css';
export type FigureProps = Omit<UIProps, 'className' | 'style'> & {
  id: string;
  pose: Pose;
  scale?: number;
  /** Frames from an imported figure pack; without it `id` names a built-in figure. */
  pack?: FigurePack;
  /** Picks the stand-in built-in figure when this one has no frame, e.g. the member id. */
  seed?: string;
};
type Natural = { src: string; width: number; height: number };
export function Figure({ id, pose, scale = 1, pack, seed, ...rest }: FigureProps) {
  // A pack frame that fails to load is remembered by URL; the built-in stand-in takes over.
  const [failed, setFailed] = useState<string>();
  // Natural pixel size by URL; the display size is worked out from the current scale each render.
  const [natural, setNatural] = useState<Natural>();
  const packed = figureFrame(id, pose, scale, pack, seed);
  const frame =
    packed && packed.src === failed ? figureFrame(id, pose, scale, undefined, seed) : packed;
  if (!frame) return null;
  const known = natural?.src === frame.src ? natural : undefined;
  const size = frame.width
    ? frame
    : known && { width: known.width * frame.factor, height: known.height * frame.factor };
  return (
    <img
      {...rest}
      className={css.root}
      src={frame.src}
      width={size?.width}
      height={size?.height}
      data-sizing={!size || undefined}
      onLoad={(e) => {
        const img = e.currentTarget;
        if (!frame.width)
          setNatural({ src: frame.src, width: img.naturalWidth, height: img.naturalHeight });
      }}
      onError={() => setFailed(frame.src)}
      alt=""
    />
  );
}
