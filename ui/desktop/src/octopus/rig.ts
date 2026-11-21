// The octopus rig: the quantised logo's squares, bound to a body and nine arms, posed and drawn back onto the logo's
// lattice in the `--octo-*` design inks. The empty pose draws the logo exactly; any pose in between two others is their
// interpolation. draw() works in the octopus's own units (960 across, origin at its centre) under whatever transform
// the context carries.

import { hash, lerp, smooth } from "./ease";
import data from "./rig.json";

/** A square: x, y, size, ink index, ink inset, whether it is in the eye's opening, its part, its arc length. */
type CellData = [number, number, number, number, number, number, number, number];
type ArmData = { name: string; depth: number; free: number; line: [number, number][] };
/** The body first, then each arm with its centreline and how much of it lies under the body. */
const R = data as unknown as {
  lattice: { x0: number; pitch: number };
  eye: [number, number];
  eyePitch: number;
  parts: [{ name: string; depth: number }, ...ArmData[]];
  cells: CellData[];
};

/** The octopus's inks, read once from the design tokens. */
type Inks = Record<(typeof INK_NAMES)[number], string>;
const INK_NAMES = ["paper", "navy", "white", "peri", "lav", "deep", "gold"] as const;
let read: Inks | null = null;
export const inks = (): Inks =>
  (read ??= Object.fromEntries(
    INK_NAMES.map((name) => [
      name,
      getComputedStyle(document.documentElement).getPropertyValue(`--octo-${name}`).trim(),
    ]),
  ) as Inks);

/**
 * How an arm bends, every field defaulting to rest. swing turns it from its root and curl adds turn growing toward the
 * tip, both in radians at the tip; wave bends it in a travelling sine moving with `phase`, each arm a little behind the
 * one before; stretch lengthens it toward the tip by that fraction of its length. heading(u) instead lays it along an
 * absolute direction at u, 0 where it leaves the body and 1 at the tip, from `from` on: the arm short of `from` keeps
 * its rest bend, and heading takes over across the next `over` of the arm.
 */
export interface ArmPose {
  swing?: number;
  curl?: number;
  wave?: number;
  phase?: number;
  stretch?: number;
  heading?: (u: number) => number;
  from?: number;
  over?: number;
}

/** Every arm's bend, and how far the body swells from its centre. */
export interface Pose {
  arms?: Record<string, ArmPose>;
  breathe?: number;
}

/**
 * How the octopus is drawn: lid 0 open to 1 shut, the way the pupil glances, -1, 0 or 1 each way, hat-driven sparkle and its seed,
 * a navy shadow offset, every square in one solid colour, or one arm drawn alone.
 */
export interface Look {
  lid?: number;
  pupil?: [number, number];
  sparkle?: number;
  seed?: number;
  shadow?: number;
  solid?: string;
  only?: string;
}

interface Arm {
  index: number;
  name: string;
  depth: number;
  free: number;
  line: [number, number][];
  heading: Float64Array;
  length: Float64Array;
  s: Float64Array;
  total: number;
  x: Float64Array;
  y: Float64Array;
  th: Float64Array;
  /** The posed centreline's length up to each point, and whether the pose stretches the arm. */
  posed: Float64Array;
  stretched: boolean;
  /** How far along its rest centreline a stretched arm keeps its own squares; the woven tentacle carries on from there. */
  kept: number;
}

interface Cell {
  index: number;
  x: number;
  y: number;
  size: number;
  color: string;
  navy: boolean;
  ink: number;
  open: boolean;
  fine: boolean;
  part: number;
  depth: number;
  k: number;
  along: number;
  across: number;
}

interface Rig {
  arms: string[];
  /** The eye's centre at rest. */
  eye: [number, number];
  root: (arm: string) => [number, number];
  tip: (pose: Pose, arm: string) => [number, number];
  /** The arm's length at rest, along its centreline. */
  length: (arm: string) => number;
  draw: (g: CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D, pose?: Pose, look?: Look) => void;
}

/** The octopus's width in its own units. */
export const ACROSS = 960;
/** The lattice's squares each way, and its first square's index left of and above the logo's own first square. */
const SPAN = 480;
const OFFSET = 240;
/** The periods of a wave along an arm, and how far each arm's wave trails the one before. */
const WAVES = 1.1;
const STAGGER = 0.29;

let made: Rig | null = null;
/** The one rig, made on first use. */
export const rig = () => (made ??= createRig(inks()));

function createRig(ink: Inks): Rig {
  /** The ink each square's index names. */
  const palette = [ink.navy, ink.white, ink.peri, ink.lav, ink.deep, ink.gold];
  const color = (i: number) => palette[i]!;
  const { navy: NAVY, gold: GOLD, peri: PERI, white: WHITE } = ink;
  const { x0: X0, pitch: P } = R.lattice;
  const EYE_PITCH = R.eyePitch;
  const [, ...armParts] = R.parts;
  const arms: Arm[] = armParts.map((part, index) => {
    const line = part.line;
    const n = line.length;
    const heading = new Float64Array(n - 1);
    const length = new Float64Array(n - 1);
    const s = new Float64Array(n);
    for (let k = 0; k < n - 1; k++) {
      const [ax, ay] = line[k]!;
      const [bx, by] = line[k + 1]!;
      heading[k] = Math.atan2(by - ay, bx - ax);
      length[k] = Math.hypot(bx - ax, by - ay);
      s[k + 1] = s[k]! + length[k]!;
    }
    for (let k = 1; k < n - 1; k++) {
      heading[k]! -= Math.round((heading[k]! - heading[k - 1]!) / (2 * Math.PI)) * 2 * Math.PI;
    }
    return {
      index,
      name: part.name,
      depth: part.depth,
      free: part.free,
      line,
      heading,
      length,
      s,
      total: s[n - 1]!,
      x: new Float64Array(n),
      y: new Float64Array(n),
      th: new Float64Array(n - 1),
      posed: new Float64Array(n),
      stretched: false,
      kept: 0,
    };
  });
  const depthOf = R.parts.map((p) => p.depth);
  /** A tentacle's half width at its tip, and where its light comes from. */
  const TIP = 0.7 * P;
  const LIGHT = [-Math.SQRT1_2, -Math.SQRT1_2] as const;

  // Each square in its part's terms: body squares by position, arm squares by segment and offset in its frame.
  const cells: Cell[] = R.cells.map(([x, y, size, c, ink, open, part, s], index) => {
    const cell: Cell = {
      index,
      x,
      y,
      size,
      color: color(c),
      navy: c === 0,
      ink,
      open: !!open,
      fine: size < P * 0.75,
      part,
      depth: depthOf[part]!,
      k: 0,
      along: 0,
      across: 0,
    };
    const arm = arms[part - 1];
    if (part && arm) {
      let k = 0;
      while (k < arm.heading.length - 1 && arm.s[k + 1]! <= s) k++;
      const [ax, ay] = arm.line[k]!;
      const c0 = Math.cos(arm.heading[k]!);
      const s0 = Math.sin(arm.heading[k]!);
      cell.k = k;
      cell.along = (x - ax) * c0 + (y - ay) * s0;
      cell.across = -(x - ax) * s0 + (y - ay) * c0;
    }
    return cell;
  });
  // One arm square of each shade a tentacle is drawn in, plain or with its navy inset.
  const shade = (ink: number, inset: boolean) =>
    cells.find((c) => c.part > 0 && !c.fine && c.color === color(ink) && c.ink > 0.3 === inset)!.index;
  const SHADE = {
    navy: shade(0, false),
    white: shade(1, false),
    lav: shade(3, false),
    lavInk: shade(3, true),
    peri: shade(2, false),
    periInk: shade(2, true),
    deepInk: shade(4, true),
  };
  const fine = cells.filter((c) => c.fine);
  const open = fine.filter((c) => c.open);
  const first = open[0]!;
  const lid = {
    top: Math.min(...open.map((c) => c.y)) - EYE_PITCH / 2,
    bottom: Math.max(...open.map((c) => c.y)) + EYE_PITCH / 2,
  };
  const slot = (x: number, y: number) =>
    `${Math.round((x - first.x) / EYE_PITCH)},${Math.round((y - first.y) / EYE_PITCH)}`;
  const openSlots = new Set(open.map((c) => slot(c.x, c.y)));
  // The pupil is the navy shape in the middle of the eye: the navy eye squares joined to the one nearest its centre.
  const pupil = (() => {
    const navy = open.filter((c) => c.navy);
    const mx = open.reduce((a, c) => a + c.x, 0) / open.length;
    const my = open.reduce((a, c) => a + c.y, 0) / open.length;
    const bySlot = new Map(navy.map((c) => [slot(c.x, c.y), c]));
    const seed = navy.reduce((a, c) => (Math.hypot(c.x - mx, c.y - my) < Math.hypot(a.x - mx, a.y - my) ? c : a));
    const found = new Set([seed]);
    const queue = [seed];
    for (let c = queue.pop(); c; c = queue.pop()) {
      for (const [dx, dy] of [
        [1, 0],
        [-1, 0],
        [0, 1],
        [0, -1],
      ] as const) {
        const n = bySlot.get(slot(c.x + dx * EYE_PITCH, c.y + dy * EYE_PITCH));
        if (n && !found.has(n)) {
          found.add(n);
          queue.push(n);
        }
      }
    }
    return [...found];
  })();
  const inPupil = new Set(pupil);
  // The pupil looks at most one eye square each way, and only as far as it stays whole inside the eye.
  const glance = ([sx, sy]: [number, number]): [number, number] => {
    const tries: [number, number][] = [
      [sx, sy],
      [sx, 0],
      [0, sy],
    ];
    const fits = ([dx, dy]: [number, number]) =>
      pupil.every((c) => openSlots.has(slot(c.x + dx * EYE_PITCH, c.y + dy * EYE_PITCH)));
    return tries.find(fits) ?? [0, 0];
  };
  const body = cells.filter((c) => c.part === 0);
  const centre = [body.reduce((a, c) => a + c.x, 0) / body.length, body.reduce((a, c) => a + c.y, 0) / body.length];

  const px = new Float64Array(cells.length);
  const py = new Float64Array(cells.length);
  let breathe = 0;
  // Each lattice square's frontmost claim this draw, valid only where `drawn` holds this draw's number, and the squares
  // claimed in the order they were first claimed: a draw touches only the squares the octopus covers.
  const winner = new Int32Array(SPAN * SPAN);
  const score = new Float64Array(SPAN * SPAN);
  const drawn = new Uint32Array(SPAN * SPAN);
  const claimed = new Int32Array(SPAN * SPAN);
  let draws = 0;
  let count = 0;
  const offer = (node: number, sc: number, i: number) => {
    if (drawn[node] !== draws) {
      drawn[node] = draws;
      claimed[count++] = node;
    } else if (sc <= score[node]!) return;
    score[node] = sc;
    winner[node] = i;
  };
  // The lattice squares the current draw fills: position, source square and lattice index.
  const nx = new Float64Array(SPAN * SPAN);
  const ny = new Float64Array(SPAN * SPAN);
  const nc = new Int32Array(SPAN * SPAN);
  const nn = new Int32Array(SPAN * SPAN);

  // Bends the arm's centreline for the pose: its heading along the arm is the rest heading plus the pose's turn,
  // which is zero up to where the arm leaves the body.
  function bend(pose: Pose, arm: Arm) {
    const {
      swing = 0,
      curl = 0,
      wave = 0,
      phase = 0,
      stretch = 0,
      heading,
      from = 0,
      over = 0.12,
    } = pose.arms?.[arm.name] ?? {};
    const lag = arm.index * STAGGER;
    arm.x[0] = arm.line[0]![0];
    arm.y[0] = arm.line[0]![1];
    const span = arm.total - arm.free;
    for (let k = 0; k < arm.heading.length; k++) {
      const mid = (arm.s[k]! + arm.s[k + 1]!) / 2;
      const u = Math.max(0, (mid - arm.free) / span);
      const ease = smooth(from, 1, u);
      const turn =
        swing * smooth(0, 0.3, u) + curl * u * u + wave * u * Math.sin(2 * Math.PI * (u * WAVES - phase - lag));
      const rest = arm.heading[k]! + turn;
      const th = heading ? lerp(rest, heading(u), smooth(from, from + over, u)) : rest;
      arm.th[k] = th;
      const len = arm.length[k]! * (1 + stretch * ease);
      arm.x[k + 1] = arm.x[k]! + len * Math.cos(th);
      arm.y[k + 1] = arm.y[k]! + len * Math.sin(th);
      arm.posed[k + 1] = arm.posed[k]! + len;
    }
    arm.stretched = Math.abs(stretch) > 1e-3;
    arm.kept = arm.free + from * span;
  }

  // The body swells from its centre as the octopus breathes.
  function swell(x: number, y: number): [number, number] {
    if (!breathe) return [x, y];
    const dx = x - centre[0]!;
    const dy = y - centre[1]!;
    const f = breathe * Math.exp(-(dx * dx + dy * dy) / (2 * 260 * 260));
    return [x + dx * f, y + dy * f];
  }

  // Places every square of `part`, or of the whole octopus when none is given.
  function place(pose: Pose, part: number | undefined) {
    for (const arm of part ? [arms[part - 1]!] : arms) bend(pose, arm);
    breathe = pose.breathe ?? 0;
    for (const c of cells) {
      const i = c.index;
      const arm = arms[c.part - 1];
      if ((part !== undefined && c.part !== part) || (arm?.stretched && arcOf(c) >= arm.kept)) {
        px[i] = NaN;
        continue;
      }
      let x = c.x;
      let y = c.y;
      if (arm) {
        const th = arm.th[c.k]!;
        x = arm.x[c.k]! + c.along * Math.cos(th) - c.across * Math.sin(th);
        y = arm.y[c.k]! + c.along * Math.sin(th) + c.across * Math.cos(th);
      }
      [px[i], py[i]] = swell(x, y);
    }
  }

  // How far along its arm's rest centreline a square sits.
  const arcOf = (c: Cell) => arms[c.part - 1]!.s[c.k]! + c.along;

  /**
   * The stretched share of an arm, past the share it keeps of its own squares, drawn as a tentacle in the logo's
   * squares rather than by stretching its own, which would smear them: it carries on at the arm's own width where the
   * kept share ends and tapers to its tip, lit on the side facing the light from the top left and shadowed on the
   * other, its dither and its row of suckers fixed to it from the tip back, so a growing arm grows out of the kept one.
   */
  function weave(arm: Arm, claim: (x: number, y: number, i: number) => void) {
    const last = arm.heading.length;
    const length = arm.posed[last]!;
    const reach = Math.max(P, length - arm.kept);
    const part = arm.index + 1;
    const near = cells.filter((c) => c.part === part && Math.abs(arcOf(c) - arm.kept) < 2 * P);
    const base = Math.min(5 * P, Math.max(2 * TIP, ...near.map((c) => Math.abs(c.across) + P / 2)));
    const half = (fromTip: number) => {
      const t = Math.min(1, Math.max(0, fromTip / reach));
      return lerp(TIP, base, t ** 0.8);
    };
    const middle = arm.th[Math.floor(last * 0.6)]!;
    const lit = -Math.sin(middle) * LIGHT[0] + Math.cos(middle) * LIGHT[1] > 0 ? 1 : -1;
    const suckers: number[] = [];
    for (let at = 2.5 * P; at < reach; at += Math.max(3 * P, 1.6 * half(at))) suckers.push(at);
    let k = 0;
    for (let along = arm.kept; along <= length; along += P / 2) {
      while (k < last - 1 && arm.posed[k + 1]! < along) k++;
      const span = arm.posed[k + 1]! - arm.posed[k]!;
      const f = span ? Math.min(1, Math.max(0, (along - arm.posed[k]!) / span)) : 0;
      const th = arm.th[k]!;
      const [cs, sn] = [Math.cos(th), Math.sin(th)];
      const cx = lerp(arm.x[k]!, arm.x[k + 1]!, f);
      const cy = lerp(arm.y[k]!, arm.y[k + 1]!, f);
      const fromTip = length - along;
      const w = half(fromTip);
      const sucker = suckers.reduce((a, b) => (Math.abs(b - fromTip) < Math.abs(a - fromTip) ? b : a), -1e9);
      for (let across = -w; across <= w; across += P / 2) {
        const [x, y] = swell(cx - across * sn, cy + across * cs);
        claim(x, y, shadeAt(fromTip, across * lit, w, sucker));
      }
    }
  }

  // The square a tentacle shows `fromTip` along it and `across` it toward the light, `half` wide each way, nearest
  // the sucker centred `sucker` from the tip.
  function shadeAt(fromTip: number, across: number, half: number, sucker: number) {
    const v = across / half;
    const h = hash(Math.round(fromTip / P) * 131 + Math.round(across / P));
    if (half - Math.abs(across) < 0.6 * P) return v > 0 ? SHADE.lav : SHADE.navy;
    const r = Math.max(1.3 * P, 0.6 * half);
    if (half >= 1.8 * P) {
      const [dx, dy] = [fromTip - sucker, across + 0.3 * half];
      const d = Math.hypot(dx, dy);
      if (d < 0.45 * r) return SHADE.navy;
      if (d < r) return dx > 0.3 * r && dy > 0.3 * r ? SHADE.white : SHADE.lav;
    }
    if (v > 0.35) return h < 0.25 ? SHADE.lavInk : SHADE.lav;
    if (v > -0.3) return h < 0.35 ? SHADE.periInk : SHADE.peri;
    return h < 0.5 ? SHADE.deepInk : SHADE.periInk;
  }

  // Snaps the posed squares back onto the lattice: every lattice square takes the frontmost posed square that lands
  // within 0.95 of a pitch of it, the nearest among equals. At rest each square lands on its own lattice square and
  // its neighbours a full pitch away, so the logo comes back exactly.
  function snap(part: number | undefined) {
    draws = (draws % 0xfffffffe) + 1;
    count = 0;
    const reach = (0.95 * P) ** 2;
    for (const c of cells) {
      if (c.fine || Number.isNaN(px[c.index])) continue;
      const i = c.index;
      const ci = Math.round((px[i]! - X0) / P);
      const cj = Math.round((py[i]! - X0) / P);
      for (let j = cj - 1; j <= cj + 1; j++) {
        for (let k = ci - 1; k <= ci + 1; k++) {
          const gi = k + OFFSET;
          const gj = j + OFFSET;
          if (gi < 0 || gj < 0 || gi >= SPAN || gj >= SPAN) continue;
          const d = (px[i]! - X0 - k * P) ** 2 + (py[i]! - X0 - j * P) ** 2;
          if (d > reach) continue;
          offer(gj * SPAN + gi, c.depth * 1e6 - d, i);
        }
      }
    }
    // A tentacle's samples sit half a pitch apart, so each claims only its nearest lattice square, at its arm's depth.
    arms.forEach((arm, a) => {
      if (!arm.stretched || (part !== undefined && part !== a + 1)) return;
      weave(arm, (x, y, i) => {
        const gi = Math.round((x - X0) / P) + OFFSET;
        const gj = Math.round((y - X0) / P) + OFFSET;
        if (gi < 0 || gj < 0 || gi >= SPAN || gj >= SPAN) return;
        const d = (x - X0 - (gi - OFFSET) * P) ** 2 + (y - X0 - (gj - OFFSET) * P) ** 2;
        offer(gj * SPAN + gi, arm.depth * 1e6 - d, i);
      });
    });
  }

  function draw(g: CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D, pose: Pose = {}, look: Look = {}) {
    const only = look.only ? armNamed(look.only).index + 1 : undefined;
    const eyes = only ? [] : fine;
    place(pose, only);
    snap(only);
    const m = g.getTransform();
    const grow = 1.3 / Math.abs(m.a);
    // Squares go down as one path per colour, which canvases fill far faster than thousands of single squares.
    const paths = new Map<string, Path2D>();
    const pathOf = (fill: string) => {
      const p = paths.get(fill) ?? new Path2D();
      paths.set(fill, p);
      return p;
    };
    const exact = (fill: string, x: number, y: number, s: number) => pathOf(fill).rect(x - s / 2, y - s / 2, s, s);
    const add = (fill: string, x: number, y: number, s: number) => exact(fill, x, y, s + grow);
    const flush = () => {
      for (const [fill, p] of paths) {
        g.fillStyle = fill;
        g.fill(p);
      }
      paths.clear();
    };
    // Only the lattice squares inside the canvas, in the octopus's own units under the context's transform.
    const back = m.inverse();
    const corners = [
      [0, 0],
      [g.canvas.width, 0],
      [0, g.canvas.height],
      [g.canvas.width, g.canvas.height],
    ].map(([x, y]) => back.transformPoint({ x, y }));
    const left = Math.min(...corners.map((c) => c.x)) - P;
    const right = Math.max(...corners.map((c) => c.x)) + P;
    const top = Math.min(...corners.map((c) => c.y)) - P;
    const bottom = Math.max(...corners.map((c) => c.y)) + P;
    // In row order, so a row's neighbours sit next to each other.
    claimed.subarray(0, count).sort();
    let n = 0;
    for (let c = 0; c < count; c++) {
      const node = claimed[c]!;
      const i = winner[node]!;
      const x = X0 + ((node % SPAN) - OFFSET) * P;
      const y = X0 + (Math.floor(node / SPAN) - OFFSET) * P;
      if (x >= left && x <= right && y >= top && y <= bottom) {
        nx[n] = x;
        ny[n] = y;
        nn[n] = node;
        nc[n++] = i;
      }
    }
    const cell = (k: number) => cells[nc[k]!]!;
    // The lattice squares in `fill(k)`'s colour, offset by `by`, each row's run of touching squares of one colour as one
    // rectangle: the canvas pays for every edge it fills, and a run has two where its squares have two each.
    const runs = (fill: (k: number) => string, by = 0) => {
      const open = new Map<string, { from: number; to: number; node: number; y: number }>();
      const side = P + grow;
      const close = (colour: string, run: { from: number; to: number; y: number }) =>
        pathOf(colour).rect(run.from - side / 2 + by, run.y - side / 2 + by, run.to - run.from + side, side);
      for (let k = 0; k < n; k++) {
        const colour = fill(k);
        const run = open.get(colour);
        if (run && nn[k] === run.node + 1) {
          run.to = nx[k]!;
          run.node = nn[k]!;
          continue;
        }
        if (run) close(colour, run);
        open.set(colour, { from: nx[k]!, to: nx[k]!, node: nn[k]!, y: ny[k]! });
      }
      for (const [colour, run] of open) close(colour, run);
    };
    // The eye moves with the body in whole eye squares, so its finer squares stay on their own lattice.
    const onEye = (posed: number, rest: number) => Math.round((posed - rest) / EYE_PITCH) * EYE_PITCH;
    const [ex, ey] = only ? [0, 0] : [onEye(px[first.index]!, first.x), onEye(py[first.index]!, first.y)];
    if (look.shadow) {
      runs(() => NAVY, look.shadow);
      for (const c of eyes) add(NAVY, c.x + ex + look.shadow, c.y + ey + look.shadow, c.size);
      flush();
    }
    if (look.solid) {
      const solid = look.solid;
      runs(() => solid);
      for (const c of eyes) add(solid, c.x + ex, c.y + ey, c.size);
      flush();
      return;
    }
    runs((k) => cell(k).color);
    flush();
    for (let k = 0; k < n; k++) if (cell(k).ink > 0.3) exact(NAVY, nx[k]!, ny[k]!, cell(k).ink);
    flush();
    if (only) return;
    for (const c of fine) if (!c.open) add(c.color, c.x + ex, c.y + ey, c.size);
    flush();
    const shut = look.lid ?? 0;
    const line = lerp(lid.top, lid.bottom, shut);
    for (const c of open) {
      const above = c.y < line;
      const fill = above ? (line - c.y < EYE_PITCH * 1.5 && shut > 0.2 ? NAVY : PERI) : inPupil.has(c) ? GOLD : c.color;
      add(fill, c.x + ex, c.y + ey, c.size);
    }
    flush();
    const [dx, dy] = glance(look.pupil ?? [0, 0]);
    for (const c of pupil) {
      if (c.y + dy * EYE_PITCH >= line) add(NAVY, c.x + dx * EYE_PITCH + ex, c.y + dy * EYE_PITCH + ey, c.size);
    }
    const sparkle = look.sparkle ?? 0;
    if (sparkle > 0.05) {
      const count = Math.round(sparkle * 70);
      for (let j = 0; j < count; j++) {
        const k = Math.floor(hash((look.seed ?? 0) * 62.3 + j) * n);
        if (!cell(k).navy) add(WHITE, nx[k]!, ny[k]!, cell(k).size);
      }
    }
    flush();
  }

  const armNamed = (name: string) => {
    const arm = arms.find((a) => a.name === name);
    if (!arm) throw new Error(`the rig has no arm ${name}`);
    return arm;
  };
  const root = (name: string): [number, number] => armNamed(name).line[0]!;
  const tip = (pose: Pose, name: string): [number, number] => {
    const arm = armNamed(name);
    const k = arm.x.length - 1;
    bend(pose, arm);
    return [arm.x[k]!, arm.y[k]!];
  };
  const length = (name: string) => armNamed(name).total;
  return { arms: arms.map((a) => a.name), eye: R.eye, root, tip, length, draw };
}
