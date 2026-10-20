export type DeskGeometry = { width: number; height: number; screens: [number, number][][] };
// Intrinsic asset size and the screen holes as corner quads (main screen first), in asset pixels.
export const desks: Record<'laptop' | 'mini', DeskGeometry> = {
  laptop: {
    width: 440,
    height: 283,
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
