export type DeskGeometry = { width: number; height: number; screens: [number, number][][] };
// Intrinsic asset dimensions. B1 will calibrate screen polygons with the scene layout.
export const desks: Record<'laptop' | 'mini', DeskGeometry> = {
  laptop: { width: 440, height: 283, screens: [] },
  mini: { width: 440, height: 280, screens: [] },
};
