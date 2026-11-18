/**
 * A tentacle: one of the logo's arms lengthened until it holds something, drawn on the octopus's own grid at its
 * half-size squares. A strand is the tentacle's reach (a smooth centreline from the arm's root to a curled tip) and the
 * arm's curl in the logo. `cellsOf` fills a strand's pose with skin: the logo's lavender and periwinkle, a navy outline,
 * and white sucker rings with navy centres along the side it holds with, shrinking toward the tip.
 */
import { ARM_ART, CELLS, LAV, NAVY, PERI, PITCH, WHITE, type ArmName, type Cell, type Ink, type Point } from "./body";
import { bezier, clamp, smooth } from "./ease";

/** World units between centreline samples. */
const DS = 6;
/** The half-size square's pitch, and its side, as much over its pitch as a full square's. */
const FINE = PITCH / 2;
const CELL = FINE * (CELLS[0]!.s / PITCH);
/** World units past an arm's root over which logo squares give way to the tentacle's. */
const BAND = 36;
/** World units along the arm's own curl a tentacle keeps before it turns for what it reaches, so it lies on the arm's base. */
const JOIN = 60;
/** How far a growing tentacle has run out: it leaves the arm's curl at rest, its tip leading, and lands softly. */
const unfurl = bezier(0.45, 0, 0.2, 1);
const full = CELLS.filter((cell) => cell.s > PITCH);
const X0 = Math.min(...full.map((cell) => cell.x));
const Y0 = Math.min(...full.map((cell) => cell.y));
const site = (i: number, j: number) => (i + 4096) * 8192 + (j + 4096);

const hash2 = (a: number, b: number) => {
  const s = Math.sin(a * 127.1 + b * 311.7) * 43758.5453;
  return s - Math.floor(s);
};
const noise = (x: number, y: number) => {
  const [i, j] = [Math.floor(x), Math.floor(y)];
  const [u, v] = [smooth(0, 1, x - i), smooth(0, 1, y - j)];
  const [a, b, c, d] = [hash2(i, j), hash2(i + 1, j), hash2(i, j + 1), hash2(i + 1, j + 1)];
  return a + (b - a) * u + (c - a) * v + (a - b - c + d) * u * v;
};
const dir = (a: number): Point => ({ x: Math.cos(a), y: Math.sin(a) });

/* ---------- centrelines ---------- */

/** A centreline as its heading every DS along it, from its start. */
type Line = { th: Float64Array; len: number };
const thAt = (line: Line, s: number) => {
  const x = clamp(s / DS, 0, line.th.length - 1);
  const i = Math.min(Math.floor(x), line.th.length - 2);
  return line.th[i]! + (line.th[i + 1]! - line.th[i]!) * (x - i);
};

function resample(points: Point[], step: number): Point[] {
  const out = [points[0]!];
  let [prev, need] = [points[0]!, step];
  for (let k = 1; k < points.length; k++) {
    const next = points[k]!;
    let segment = Math.hypot(next.x - prev.x, next.y - prev.y);
    while (segment >= need) {
      const f = need / segment;
      prev = { x: prev.x + (next.x - prev.x) * f, y: prev.y + (next.y - prev.y) * f };
      out.push(prev);
      segment -= need;
      need = step;
    }
    need -= segment;
    prev = next;
  }
  return out;
}

/** The polyline with every corner rounded off, so its curvature varies continuously; its two ends stay put. */
function relax(points: Point[], sigma: number): Point[] {
  const p = resample(points, 3);
  const [n, r] = [p.length, Math.ceil(sigma)];
  const at = (i: number): Point => {
    if (i < 0) return { x: p[0]!.x + (p[0]!.x - p[1]!.x) * -i, y: p[0]!.y + (p[0]!.y - p[1]!.y) * -i };
    if (i >= n) {
      const [a, b] = [p[n - 1]!, p[n - 2]!];
      return { x: a.x + (a.x - b.x) * (i - n + 1), y: a.y + (a.y - b.y) * (i - n + 1) };
    }
    return p[i]!;
  };
  const weights = Array.from({ length: 2 * r + 1 }, (_, k) => Math.exp(-(((k - r) * 3) ** 2) / (2 * sigma * sigma)));
  return p.map((_, i) => {
    let [x, y, w] = [0, 0, 0];
    for (let k = -r; k <= r; k++) {
      const [g, q] = [weights[k + r]!, at(i + k)];
      x += g * q.x;
      y += g * q.y;
      w += g;
    }
    return { x: x / w, y: y / w };
  });
}

function lineOf(points: Point[]): Line {
  const p = resample(points, DS);
  const th = new Float64Array(p.length);
  for (let i = 0; i < p.length - 1; i++) {
    let a = Math.atan2(p[i + 1]!.y - p[i]!.y, p[i + 1]!.x - p[i]!.x);
    if (i) a -= Math.round((a - th[i - 1]!) / (2 * Math.PI)) * 2 * Math.PI;
    th[i] = a;
  }
  th[p.length - 1] = th[p.length - 2]!;
  return { th, len: (p.length - 1) * DS };
}

/** A smooth curve through the points, leaving the first along `heading`. */
function spline(points: Point[], heading: number): Point[] {
  const h = dir(heading);
  const last = points[points.length - 1]!;
  const p = [{ x: points[0]!.x - h.x * 30, y: points[0]!.y - h.y * 30 }, ...points, last];
  const out: Point[] = [];
  for (let i = 1; i < p.length - 2; i++)
    for (let k = 0; k < 12; k++) {
      const [u, a, b, c, d] = [k / 12, p[i - 1]!, p[i]!, p[i + 1]!, p[i + 2]!];
      const f = (a: number, b: number, c: number, d: number) =>
        0.5 * (2 * b + (c - a) * u + (2 * a - 5 * b + 4 * c - d) * u * u + (3 * b - a - 3 * c + d) * u ** 3);
      out.push({ x: f(a.x, b.x, c.x, d.x), y: f(a.y, b.y, c.y, d.y) });
    }
  out.push(last);
  return out;
}

/* ---------- the logo's arms ---------- */

/**
 * An arm a tentacle grows from: its root, its curl at rest, the logo squares the tentacle stands in for (`hidden`) and
 * the full-size grid sites just past the root that keep their logo square, so the join has no edge (`kept`).
 */
export type Arm = { root: Point; heading: number; half: number; rest: Line; hidden: Set<Cell>; kept: Set<number> };

function armFrom(name: ArmName): Arm {
  const art = ARM_ART[name];
  const [root, half] = [{ x: art.root[0], y: art.root[1] }, art.width / 2];
  const rest = lineOf(relax(spline([root, ...art.curl.map(([x, y]) => ({ x, y }))], art.heading), 6));
  const line = walk(root, rest, rest.len);
  const [hidden, kept] = [new Set<Cell>(), new Set<number>()];
  /**
   * Squares of the arm's base, where the tentacle lies on the arm's own curl: those on the body's side of it, and those
   * on the curl's side wider than the tentacle. They stay with the body, under the tentacle where it covers them.
   */
  const base = new Set<Cell>();
  const curlSide = Math.sign(thAt(rest, rest.len / 2) - thAt(rest, 0));
  for (const cell of CELLS) {
    let [best, nearest] = [0, Infinity];
    for (let i = 0; i <= line.n; i++) {
      const d = (cell.x - line.x[i]!) ** 2 + (cell.y - line.y[i]!) ** 2;
      if (d < nearest) [nearest, best] = [d, i];
    }
    const [t, rx, ry] = [dir(line.th[best]!), cell.x - line.x[best]!, cell.y - line.y[best]!];
    const along = rx * t.x + ry * t.y;
    const [s, across] = [line.s[best]! + along, ry * t.x - rx * t.y];
    const d = Math.abs(across);
    const width = Math.min(half, 5 + (1.05 * half * (rest.len - s)) / rest.len) + 7;
    if (s < 0 || d > width || (best === line.n && along > half)) continue;
    if (s < JOIN + PITCH && (across * curlSide < 0 || d > half - FINE)) base.add(cell);
    const [i, j] = [Math.round((cell.x - X0) / PITCH), Math.round((cell.y - Y0) / PITCH)];
    if (s < BAND * hash2(i, j)) kept.add(site(i, j));
    else hidden.add(cell);
  }
  // squares of the curl that the centreline missed are left joined to nothing: they go with it
  const grid = new Map<number, Cell[]>();
  const key = (cell: Cell) => site(Math.round(cell.x / PITCH), Math.round(cell.y / PITCH));
  for (const cell of CELLS) if (!hidden.has(cell)) grid.set(key(cell), [...(grid.get(key(cell)) ?? []), cell]);
  const centre = CELLS.reduce((a, b) => (Math.hypot(b.x, b.y) < Math.hypot(a.x, a.y) ? b : a));
  const joined = new Set<Cell>([centre]);
  const queue = [centre];
  while (queue.length) {
    const cell = queue.pop()!;
    const [i, j] = [Math.round(cell.x / PITCH), Math.round(cell.y / PITCH)];
    for (let dj = -2; dj <= 2; dj++)
      for (let di = -2; di <= 2; di++)
        for (const next of grid.get(site(i + di, j + dj)) ?? []) {
          const near = Math.abs(next.x - cell.x) < PITCH * 1.5 && Math.abs(next.y - cell.y) < PITCH * 1.5;
          if (near && !joined.has(next)) (joined.add(next), queue.push(next));
        }
  }
  for (const cell of CELLS) if (!joined.has(cell)) hidden.add(cell);
  for (const cell of base) hidden.delete(cell);
  return { root, heading: art.heading, half, rest, hidden, kept };
}

const arms = new Map<ArmName, Arm>();
/** The arm `name`, worked out the first time a tentacle grows from it. */
export function armOf(name: ArmName): Arm {
  const known = arms.get(name) ?? armFrom(name);
  arms.set(name, known);
  return known;
}

/* ---------- what a tentacle holds ---------- */

export type Corner = "topLeft" | "topRight" | "bottomLeft" | "bottomRight";
/**
 * The last stretch of a tentacle, which it lays on what it holds: its points, the heading it arrives on, the point it
 * holds, and its length.
 */
export type Hold = { points: Point[]; heading: number; at: Point; length: number };

const HOOK = 17;
/** The tip curling on, `turn` 1 clockwise on screen, from `end` along heading `h`. */
function hook(end: Point, h: Point, turn: number): Point[] {
  const centre = { x: end.x - h.y * turn * HOOK, y: end.y + h.x * turn * HOOK };
  const points: Point[] = [];
  for (let a = 0; a < 2.3; a += 3 / HOOK) {
    const angle = Math.atan2(end.y - centre.y, end.x - centre.x) + turn * a;
    points.push({ x: centre.x + HOOK * Math.cos(angle), y: centre.y + HOOK * Math.sin(angle) });
  }
  return points;
}

/**
 * Holding a rect (corners rounded at `radius`) by `corner`: the tentacle runs along one edge to the corner, turns round
 * it (`turn` 1 clockwise on screen) and curls its tip onto the next edge.
 */
export function corner(rect: { x: number; y: number; w: number; h: number }, radius: number, at: Corner, turn: 1 | -1) {
  const [sx, sy] = [at.endsWith("Left") ? -1 : 1, at.startsWith("top") ? -1 : 1];
  const rc = Math.max(radius, 26);
  const c = { x: at.endsWith("Left") ? rect.x : rect.x + rect.w, y: at.startsWith("top") ? rect.y : rect.y + rect.h };
  const k = { x: c.x - sx * rc, y: c.y - sy * rc };
  const [phiX, phiY, quarter] = [sx > 0 ? 0 : Math.PI, (sy * Math.PI) / 2, (turn * Math.PI) / 2];
  const from = Math.abs(Math.sin((phiX + quarter - phiY) / 2)) < 1e-6 ? phiX : phiY;
  const to = from + quarter;
  const [rho, lead, run] = [rc + 4, 70, 30];
  const on = (phi: number) => ({ x: k.x + rho * Math.cos(phi), y: k.y + rho * Math.sin(phi) });
  const [h0, h1, a, b] = [dir(from + quarter), dir(to + quarter), on(from), on(to)];
  const points: Point[] = [];
  for (let d = lead; d > 0; d -= 3) points.push({ x: a.x - h0.x * d, y: a.y - h0.y * d });
  for (let phi = 0; phi < Math.abs(to - from); phi += 3 / rho) points.push(on(from + turn * phi));
  for (let d = 0; d < run; d += 3) points.push({ x: b.x + h1.x * d, y: b.y + h1.y * d });
  points.push(...hook({ x: b.x + h1.x * run, y: b.y + h1.y * run }, h1, turn));
  const mid = from + quarter / 2;
  return {
    points,
    heading: from + quarter,
    at: { x: k.x + rc * Math.cos(mid), y: k.y + rc * Math.sin(mid) },
    length: lead + rho * Math.abs(to - from) + run + HOOK * 2.3,
  } satisfies Hold;
}

/** Hooking an edge at `at`, arriving along `heading` and curling its tip round behind it (`turn` 1 clockwise). */
export function edge(at: Point, heading: number, turn: 1 | -1): Hold {
  const [h, lead] = [dir(heading), 120];
  const points: Point[] = [];
  for (let d = lead; d > 0; d -= 3) points.push({ x: at.x - h.x * d, y: at.y - h.y * d });
  points.push(at, ...hook(at, h, turn));
  return { points, heading, at, length: lead + HOOK * 2.3 };
}

export type Strand = {
  root: Point;
  path: Line;
  rest: Line;
  half: number;
  turn: number;
  arm: Arm;
  /** How much of the path's end lies on what it holds. */
  holding: number;
  suckers: { s: number; r: number }[];
  near: Int16Array;
  seed: number;
};

const bandWidth = (half: number) => clamp(0.5 * (2 * half - 13), 0, 40);

/** The strand from `arm` to what it holds. */
export function reach(arm: Arm, hold: Hold, turn: 1 | -1, seed: number): Strand {
  const base = walk(arm.root, arm.rest, JOIN);
  const along: Point[] = Array.from({ length: base.n + 1 }, (_, i) => ({ x: base.x[i]!, y: base.y[i]! }));
  const start = along[base.n]!;
  const entry = hold.points[0]!;
  const d = Math.hypot(entry.x - start.x, entry.y - start.y);
  const [h, hg] = [dir(base.th[base.n]!), dir(hold.heading)];
  const p1 = { x: start.x + h.x * d * 0.33, y: start.y + h.y * d * 0.33 };
  const p2 = { x: entry.x - hg.x * d * 0.28, y: entry.y - hg.y * d * 0.28 };
  const points: Point[] = [];
  for (let i = 1; i < 240; i++) {
    const [u, v] = [i / 240, 1 - i / 240];
    const [a, b, c, e] = [v ** 3, 3 * v * v * u, 3 * v * u * u, u ** 3];
    points.push({
      x: a * start.x + b * p1.x + c * p2.x + e * entry.x,
      y: a * start.y + b * p1.y + c * p2.y + e * entry.y,
    });
  }
  const path = lineOf(relax([...along, ...points, ...hold.points], 9));
  const suckers: Strand["suckers"] = [];
  for (let s = 50; s < path.len - 12;) {
    const band = bandWidth(arm.half * (1 - (0.4 * s) / path.len)) * smooth(40, 160, s);
    if (band < 4) {
      s += PITCH;
      continue;
    }
    suckers.push({ s: s + band / 2, r: band / 2 });
    s += band + 5;
  }
  const near = new Int16Array(Math.ceil(path.len / DS) + 1).fill(-1);
  let k = 0;
  for (let i = 0; i < near.length && suckers.length; i++) {
    while (k < suckers.length - 1 && Math.abs(suckers[k + 1]!.s - i * DS) < Math.abs(suckers[k]!.s - i * DS)) k++;
    near[i] = k;
  }
  return { root: arm.root, path, rest: arm.rest, half: arm.half, turn, arm, holding: hold.length, suckers, near, seed };
}

/* ---------- a moment ---------- */

/**
 * A strand's pose: its length, how much of the arm's curl its tip still carries (1 at rest), how far the tip has opened
 * away from what it holds (radians) and the sideways swing of its middle (world units).
 */
export type Moment = { len: number; curl: number; open: number; whip: number };

/**
 * Growing, x from 0 to 1 and on: the tip runs out along the path, uncurling, and arrives at x = 1; the middle trails to
 * one side on the way, swings past once the tip has landed, and settles.
 */
export function reaching(st: Strand, x: number): Moment {
  const [len0, swing, e] = [st.rest.len, 0.03 * st.path.len * st.turn, unfurl(Math.min(x, 1))];
  return {
    len: len0 + (st.path.len - len0) * e,
    curl: 1 - smooth(0, 0.6, x),
    open: 0,
    whip: swing * (x < 1 ? -Math.sin(Math.PI * e) : 0.45 * Math.exp(-3 * (x - 1)) * Math.sin(2.2 * Math.PI * (x - 1))),
  };
}

/** Letting go, y from 0 to 1: the tip opens off what it held, the tentacle whips back and shortens into its arm, curling. */
export function recoiling(st: Strand, from: Moment, y: number): Moment {
  const len0 = st.rest.len;
  return {
    len: from.len + (len0 - from.len) * smooth(0.1, 1, y),
    curl: from.curl + (1 - from.curl) * smooth(0.35, 1, y),
    open: from.open * (1 - y) + 1.2 * Math.sin(Math.PI * clamp(y / 0.45, 0, 1)),
    whip: from.whip * (1 - y) - 0.024 * from.len * st.turn * Math.sin(Math.PI * y),
  };
}

/**
 * Releasing, y from 0 to 1: the tip opens off what it held and draws `back` along the path, to rest a little lifted
 * with its middle swung once and settled.
 */
export function releasing(st: Strand, back: number, y: number): Moment {
  return {
    len: st.path.len - back * smooth(0, 0.8, y),
    curl: 0,
    open: 0.9 * Math.sin(Math.PI * clamp(y / 0.55, 0, 1)) + 0.1 * smooth(0.3, 1, y),
    whip: -0.018 * st.path.len * st.turn * Math.sin(Math.PI * y),
  };
}

type Walk = { x: Float64Array; y: Float64Array; th: Float64Array; s: Float64Array; n: number };
function walk(root: Point, line: Line, len: number, bend?: (s: number) => number, whip = 0): Walk {
  const n = Math.max(1, Math.ceil(len / DS));
  const w: Walk = {
    x: new Float64Array(n + 1),
    y: new Float64Array(n + 1),
    th: new Float64Array(n + 1),
    s: new Float64Array(n + 1),
    n,
  };
  let [x, y] = [root.x, root.y];
  for (let i = 0; i <= n; i++) {
    const s = Math.min(i * DS, len);
    const th = thAt(line, s) + (bend ? bend(s) : 0);
    [w.x[i], w.y[i], w.th[i], w.s[i]] = [x, y, th, s];
    const step = Math.min(DS, len - s);
    x += Math.cos(th) * step;
    y += Math.sin(th) * step;
  }
  if (whip)
    for (let i = 0; i <= n; i++) {
      const k = whip * Math.sin((Math.PI * w.s[i]!) / len) ** 2;
      w.x[i] = w.x[i]! - Math.sin(w.th[i]!) * k;
      w.y[i] = w.y[i]! + Math.cos(w.th[i]!) * k;
    }
  return w;
}

const shape = (st: Strand, m: Moment) =>
  walk(
    st.root,
    st.path,
    m.len,
    (s) => {
      const q = m.len - s;
      const curl =
        m.curl && q < st.rest.len ? m.curl * (thAt(st.rest, st.rest.len - q) - thAt(st.path, st.rest.len - q)) : 0;
      return curl - st.turn * m.open * Math.exp(-q / 120);
    },
    m.whip,
  );

/** Half the tentacle's width at s: a steady taper from the arm's width, and thin over its last stretch to the tip. */
const halfAt = (st: Strand, s: number, len: number) =>
  Math.min(st.half * (1 - (0.4 * s) / st.path.len), 5 + (len - s) * Math.max(0.06, (1.05 * st.half) / len));

/** Where the strand's tip is at a moment. */
export function tipOf(st: Strand, m: Moment): Point {
  const w = shape(st, m);
  return { x: w.x[w.n]!, y: w.y[w.n]! };
}

/* ---------- skin ---------- */

/** Receives one square of a strand: its centre and side in world units, its ink, and its length along the strand. */
export type EmitAlong = (x: number, y: number, size: number, ink: Ink, s: number) => void;

/** The strand's squares at a moment: half-size squares on the octopus's grid, then the navy dots that sit on them. */
export function cellsOf(st: Strand, m: Moment, emit: EmitAlong) {
  if (m.len < 1) return;
  const [w, len, xf, yf] = [shape(st, m), m.len, X0 - FINE / 2, Y0 - FINE / 2];
  let [minX, minY, maxX, maxY] = [Infinity, Infinity, -Infinity, -Infinity];
  for (let i = 0; i <= w.n; i++) {
    [minX, maxX] = [Math.min(minX, w.x[i]!), Math.max(maxX, w.x[i]!)];
    [minY, maxY] = [Math.min(minY, w.y[i]!), Math.max(maxY, w.y[i]!)];
  }
  const pad = st.half + 3 * FINE;
  const [i0, j0] = [Math.floor((minX - pad - xf) / FINE), Math.floor((minY - pad - yf) / FINE)];
  const gw = Math.ceil((maxX + pad - xf) / FINE) - i0 + 1;
  const gh = Math.ceil((maxY + pad - yf) / FINE) - j0 + 1;
  const best = new Float32Array(gw * gh).fill(Infinity);
  const [S, D] = [new Float32Array(gw * gh), new Float32Array(gw * gh)];
  for (let k = 0; k <= w.n; k++) {
    const [r, px, py] = [halfAt(st, w.s[k]!, len) + 8, w.x[k]!, w.y[k]!];
    const [tx, ty] = [Math.cos(w.th[k]!), Math.sin(w.th[k]!)];
    const [ia, ib] = [Math.ceil((px - r - xf) / FINE), Math.floor((px + r - xf) / FINE)];
    const [ja, jb] = [Math.ceil((py - r - yf) / FINE), Math.floor((py + r - yf) / FINE)];
    for (let j = ja; j <= jb; j++)
      for (let i = ia; i <= ib; i++) {
        const [rx, ry] = [xf + i * FINE - px, yf + j * FINE - py];
        const [d2, at] = [rx * rx + ry * ry, (j - j0) * gw + (i - i0)];
        if (d2 >= best[at]!) continue;
        best[at] = d2;
        S[at] = w.s[k]! + rx * tx + ry * ty;
        D[at] = ry * tx - rx * ty;
      }
  }
  // 1 inside the tentacle, 2 before its root (the arm it grows from), 0 outside
  const tip = halfAt(st, len, len);
  const state = new Uint8Array(gw * gh);
  for (let at = 0; at < state.length; at++) {
    if (best[at] === Infinity) continue;
    const [s, d] = [S[at]!, Math.abs(D[at]!)];
    if (s < 0) state[at] = d <= st.half + FINE ? 2 : 0;
    else if (s > len) state[at] = (s - len) ** 2 + d * d <= tip * tip ? 1 : 0;
    else state[at] = d <= halfAt(st, s, len) ? 1 : 0;
  }
  const dots: [number, number, number, number][] = [];
  for (let j = 0; j < gh; j++)
    for (let i = 0; i < gw; i++) {
      const at = j * gw + i;
      if (state[at] !== 1) continue;
      const [gi, gj] = [i + i0, j + j0];
      const [pi, pj] = [Math.floor(gi / 2), Math.floor(gj / 2)];
      if (st.arm.kept.has(site(pi, pj))) continue;
      const [s, x, y] = [S[at]!, xf + gi * FINE, yf + gj * FINE];
      let rim = false;
      for (let dj = -1; dj <= 1 && !rim; dj++)
        for (let di = -1; di <= 1; di++) {
          const [ni, nj] = [i + di, j + dj];
          const n = nj * gw + ni;
          const outside = ni < 0 || nj < 0 || ni >= gw || nj >= gh || state[n] === 0;
          if (outside || (state[n] === 1 && Math.abs(S[n]! - s) > 90)) {
            rim = true;
            break;
          }
        }
      if (rim) {
        emit(x, y, CELL, NAVY, s);
        continue;
      }
      const [half, toward] = [halfAt(st, s, len), st.turn * D[at]!];
      const band = bandWidth(half) * smooth(40, 160, s) * smooth(10, 50, len - s);
      if (band >= 4 && toward > half - FINE - band) {
        let ink = NAVY;
        const k = st.near[clamp(Math.round(s / DS), 0, st.near.length - 1)]!;
        for (let q = Math.max(0, k - 1); q <= Math.min(st.suckers.length - 1, k + 1) && k >= 0; q++) {
          const r = Math.min(st.suckers[q]!.r, band / 2);
          if (Math.hypot(s - st.suckers[q]!.s, toward - (half - FINE - band / 2)) <= Math.max(r, 4.3)) ink = WHITE;
        }
        emit(x, y, CELL, ink, s);
        continue;
      }
      // skin is coloured by the full-size square the half-size one belongs to
      const home = (pj * 2 - j0) * gw + (pi * 2 - i0);
      const inHome = home >= 0 && state[home] === 1;
      const [hs, hd] = [inHome ? S[home]! : s, inHome ? D[home]! : D[at]!];
      const shade = noise(hs / 30 + st.seed, hd / 30) + (0.3 * st.turn * hd) / half;
      emit(x, y, CELL, shade > 0.6 ? PERI : LAV, s);
      if (at === home && hash2(pi + st.seed, pj) < 0.08 + 0.14 * Math.max(0, (st.turn * hd) / half))
        dots.push([x + FINE / 2, y + FINE / 2, 4.5, s]);
    }
  // each sucker's navy centre
  for (const sucker of st.suckers) {
    if (sucker.s > len - 20) break;
    const half = halfAt(st, sucker.s, len);
    const band = bandWidth(half) * smooth(40, 160, sucker.s) * smooth(10, 50, len - sucker.s);
    const r = Math.min(sucker.r, band / 2);
    if (r < 5) continue;
    const [i, off] = [Math.round(sucker.s / DS), st.turn * (half - FINE - band / 2)];
    const [x, y] = [w.x[i]! - Math.sin(w.th[i]!) * off, w.y[i]! + Math.cos(w.th[i]!) * off];
    dots.push([
      xf + Math.round((x - xf) / 3) * 3,
      yf + Math.round((y - yf) / 3) * 3,
      clamp(r * 0.75, 3.5, 9),
      sucker.s,
    ]);
  }
  for (const [x, y, size, s] of dots) emit(x, y, size, NAVY, s);
}
