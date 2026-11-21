// The display cut into tiles that turn over: each tile's front is its square of the bare desktop and its back its
// square of the octopus asleep on its paper.

import { byOpacity, C, COLS, cut, display, DPR, ROWS, T, TH, type Context, type Picture } from "./display";
import { bare, reveal } from "./stills";

export const tileCenter = (col: number, row: number): [number, number] => [col * T + T / 2, row * TH + TH / 2];
// The tile's centre in global points, so a wave crosses displays in one piece.
export const tileGlobal = (col: number, row: number) => {
  const [x, y] = tileCenter(col, row);
  return [display.x + x / DPR, display.y + y / DPR] as const;
};

export function eachTile(fn: (col: number, row: number, i: number) => void) {
  for (let row = 0; row < ROWS; row++) for (let col = 0; col < COLS; col++) fn(col, row, row * COLS + col);
}

/** A tile placed at (x, y), `s` of its size, turned over by th (0 front, PI back) about a vertical axis, or a
 * horizontal one when `across`. */
interface Turned {
  x: number;
  y: number;
  s?: number;
  th?: number;
  across?: boolean;
}

/** A tile as drawn: its centre lifted as it turns, its length along the turn and its width across it, and the picture
 * its face is cut from. */
export interface Shape {
  col: number;
  row: number;
  cx: number;
  cy: number;
  across: boolean;
  span: number;
  wide: number;
  lift: number;
  cos: number;
  depth: number;
  strips: number;
  img: Picture;
}

// Mid-turn a tile is drawn in up to four strips that shrink toward its far edge, so it reads as a card in perspective.
export function shapeOf(col: number, row: number, { x, y, s = 1, th = 0, across = false }: Turned): Shape {
  const cos = Math.cos(th);
  const sin = Math.sin(th);
  const lift = Math.abs(sin);
  const grow = s * (1 + 0.2 * lift);
  // Along the turn and across it: x and y for a vertical axis, y and x for a horizontal one.
  const [long, wide] = across ? [TH * grow, T * grow] : [T * grow, TH * grow];
  return {
    col,
    row,
    cx: x,
    cy: y - lift * TH * 0.12,
    across,
    span: long * Math.abs(cos),
    wide,
    lift,
    cos,
    depth: 0.35 * sin * Math.sign(cos || 1),
    strips: lift > 0.05 ? Math.min(4, Math.max(2, Math.round(4 * lift * s))) : 1,
    img: cos >= 0 ? bare : reveal,
  };
}
// The device point `u` along the tile's turn and `v` across it from its centre.
const onTile = (t: Shape, u: number, v: number): [number, number] =>
  t.across ? [t.cx + v, t.cy + u] : [t.cx + u, t.cy + v];

/** Navy veils by opacity, so a frame of many tiles fills a few paths. */
type Veils = ReturnType<typeof byOpacity>;
function paintVeils(g: Context, veils: Veils) {
  g.fillStyle = C.navy;
  for (const [alpha, path] of veils.paths) {
    g.globalAlpha = alpha;
    g.fill(path);
  }
  g.globalAlpha = 1;
}

function tileShadow(t: Shape, veils: Veils) {
  if (t.lift <= 0.02) return;
  const [x, y] = onTile(t, -t.span / 2, -t.wide / 2);
  const [w, h] = t.across ? [t.wide, t.span] : [t.span, t.wide];
  veils.at(0.5 * t.lift)?.rect(x + t.lift * T * 0.16, y + t.lift * TH * 0.28, w, h);
}

function tileFace(g: Context, t: Shape) {
  const [sx, sy, n] = [t.col * T, t.row * TH, t.strips];
  for (let i = 0; i < n; i++) {
    const tall = t.wide * (1 + t.depth * 2 * ((i + 0.5) / n - 0.5));
    const a = -t.span / 2 + (i * t.span) / n;
    if (t.across) cut(g, t.img, sx, sy + (i * TH) / n, T, TH / n, t.cx - tall / 2, t.cy + a, tall, t.span / n + 0.6);
    else cut(g, t.img, sx + (i * T) / n, sy, T / n, TH, t.cx + a, t.cy - tall / 2, t.span / n + 0.6, tall);
  }
}

function tileShade(t: Shape, veils: Veils) {
  if (t.cos < 0 || t.cos >= 0.98) return;
  const path = veils.at(0.75 * (1 - t.cos));
  if (!path) return;
  const near = t.wide * (1 - t.depth);
  const far = t.wide * (1 + t.depth);
  path.moveTo(...onTile(t, -t.span / 2, -near / 2));
  path.lineTo(...onTile(t, t.span / 2, -far / 2));
  path.lineTo(...onTile(t, t.span / 2, far / 2));
  path.lineTo(...onTile(t, -t.span / 2, near / 2));
  path.closePath();
}

// Tiles in the order given, all their shadows first, then their faces, then their shades, each a navy veil whose
// opacity follows the turn.
export function tiles(g: Context, shapes: Shape[]) {
  const shadows = byOpacity(32);
  const shades = byOpacity(32);
  for (const t of shapes) tileShadow(t, shadows);
  paintVeils(g, shadows);
  for (const t of shapes) {
    tileFace(g, t);
    tileShade(t, shades);
  }
  paintVeils(g, shades);
}
