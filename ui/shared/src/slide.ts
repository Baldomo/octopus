/** An item's box within its list, in the layout's pixels. */
export type Box = { x: number; y: number; width: number; height: number };

/** The parts of a 2D transform drawn from a box's top-left corner: its scale `a`, `d` and its shift `e`, `f`. */
type Drawn = { a: number; d: number; e: number; f: number };

export const shownAt = (box: Box, transform: Drawn): Box => ({
  x: box.x + transform.e,
  y: box.y + transform.f,
  width: box.width * transform.a,
  height: box.height * transform.d,
});

/**
 * The transform, drawn from the top-left corner, that shows an item laid out at `to` where it stood at `from`: shifted
 * there, and stretched to that size too where it `stretches`, so every edge of it slides on one curve.
 */
export const slideFrom = (from: Box, to: Box, stretches: boolean) => ({
  a: stretches ? from.width / to.width : 1,
  d: stretches ? from.height / to.height : 1,
  e: from.x - to.x,
  f: from.y - to.y,
});

export const moves = (from: Box, to: Box, stretches: boolean) =>
  [from.x - to.x, from.y - to.y, ...(stretches ? [from.width - to.width, from.height - to.height] : [])].some(
    (change) => Math.abs(change) >= 0.5,
  );
