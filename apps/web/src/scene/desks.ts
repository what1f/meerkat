import type { CSSProperties } from 'react';
export type DeskGeometry = {
  width: number;
  height: number;
  screens: [number, number][][];
  // Where the mug stands, in asset pixels.
  mug: [number, number];
  // A small plant where the art has room for one.
  plant?: [number, number];
};
// Intrinsic asset size and the screen holes as corner quads (main screen first), in asset pixels.
export const desks: Record<'laptop' | 'mini', DeskGeometry> = {
  laptop: {
    width: 440,
    height: 283,
    mug: [336, 112],
    plant: [372, 96],
    screens: [
      [
        [145, 6],
        [297, 6],
        [296, 89],
        [146, 89],
      ],
      [
        [39, 41],
        [112, 37],
        [116, 84],
        [42, 90],
      ],
    ],
  },
  mini: {
    width: 440,
    height: 280,
    mug: [104, 122],
    screens: [
      [
        [127, 5],
        [315, 5],
        [315, 92],
        [127, 92],
      ],
    ],
  },
};

// Each screen is clipped to its hole so nothing shows through gaps elsewhere in the art.
export const screenRects = (desk: DeskGeometry, k: number, top: number) =>
  desk.screens.map((quad) => {
    const xs = quad.map(([x]) => x * k);
    const ys = quad.map(([, y]) => y * k);
    const l = Math.min(...xs) - 1;
    const t = Math.min(...ys) - 1;
    const clip = quad
      .map(([x, y], c) => `${x * k - l + [-1, 1, 1, -1][c]!}px ${y * k - t + [-1, -1, 1, 1][c]!}px`)
      .join(',');
    return {
      left: l,
      top: top + t,
      width: Math.max(...xs) - l + 1,
      height: Math.max(...ys) - t + 1,
      clipPath: `polygon(${clip})`,
    } satisfies CSSProperties;
  });
