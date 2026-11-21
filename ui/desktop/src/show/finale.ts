// The finale: the octopus wipes its ink off the screens with six of its own arms, coming in from every side out of
// step with each other. Each slides in from beyond an edge, sweeps across like a wiper and slides back out. Its bend
// comes from how fast it swings: it lags against the motion, more toward the tip, its tip hooks tighter as it slows,
// and a wave runs down it, so it whips without ever folding over. Each arm clears what it covers and everything its
// line sweeps past out to its tip; together the sweeps cover the whole display, and leave faint wet streaks along the
// arcs the tips traced, which dry out in the order they were wiped.

import { clamp, hash, inOut, lerp, outCubic, smooth } from "../octopus/ease";
import type { Look, Pose } from "../octopus/rig";
import { RIG } from "./creature";
import { BLACK, bounds, byOpacity, C, DPR, H, layer, lx, ly, T, W, type Context, type Picture } from "./display";
import { AT } from "./music";

// `from` is where the arm pivots, as a share of the display across and down; `reach` its length as a share of the
// display's height; `sweep` its direction in degrees at the start and end (0 points right, 90 down); `at` and `lasts`
// when its stroke starts after the finale does and how long it takes, in seconds; `side` which way its tip hooks.
interface Wiper {
  arm: string;
  from: [number, number];
  reach: number;
  sweep: [number, number];
  at: number;
  lasts: number;
  side: number;
  seed: number;
}
const WIPERS: Wiper[] = [
  { arm: "F", from: [0, 1], reach: 1.35, sweep: [-4, -86], at: 0.02, lasts: 0.95, side: 1, seed: 0.1 },
  { arm: "E", from: [1, 0], reach: 1.3, sweep: [176, 94], at: 0.22, lasts: 1.05, side: -1, seed: 0.5 },
  { arm: "A", from: [0, 0.3], reach: 1.0, sweep: [-75, 70], at: 0.5, lasts: 1.0, side: -1, seed: 0.8 },
  { arm: "D", from: [1, 0.72], reach: 1.0, sweep: [-105, -255], at: 0.68, lasts: 1.1, side: 1, seed: 0.3 },
  { arm: "C", from: [0.42, 0], reach: 0.95, sweep: [8, 172], at: 0.95, lasts: 0.9, side: 1, seed: 0.6 },
  { arm: "B", from: [0.6, 1], reach: 0.95, sweep: [-172, -8], at: 1.15, lasts: 0.95, side: -1, seed: 0.9 },
];
// Each stroke slides in over IN, sweeps over its `lasts`, and slides back out over OUT.
const IN = 0.3;
const OUT = 0.35;

/** The ink as it stands when the finale starts, and everything the arms have swept since. */
let inked: Picture;
let wiped: OffscreenCanvas;
let sweeps: Map<Wiper, (Sample | null)[]>;
let trails: Trail[];

/** Takes the ink the arms wipe and builds every arm's stroke and the streaks they leave; hands back the pictures to
 * warm. */
export function prepareFinale(ink: Picture): CanvasImageSource[] {
  inked = ink;
  wiped = layer();
  wipedTo = AT.back - WIPE_STEP;
  sweeps = sweep();
  trails = streaked();
  return [inked, wiped];
}

export function finale(g: Context, t: number) {
  stamp(t);
  g.clearRect(0, 0, W, H);
  g.drawImage(inked, 0, 0);
  g.globalCompositeOperation = "destination-out";
  g.drawImage(wiped, 0, 0);
  g.globalCompositeOperation = "source-over";
  streaks(g, t);
  spray(g, t);
  for (const w of WIPERS) wiper(g, w, t, { shadow: 10 });
}

const strokeAge = (w: Wiper, t: number) => t - AT.back - w.at;
// How far in from beyond its edge an arm is (0 to 1), and the direction it points, in radians.
const reachOf = (w: Wiper, t: number) => {
  const v = strokeAge(w, t);
  return outCubic(v / IN) * (1 - inOut((v - w.lasts) / OUT));
};
const baseAngle = (w: Wiper, t: number) => {
  const u = inOut((strokeAge(w, t) - IN * 0.4) / w.lasts);
  return (lerp(w.sweep[0], w.sweep[1], u) * Math.PI) / 180;
};

// The arm's pose at t and the transform from rig units to device pixels. Its root sits beyond the edge, pulled back
// along the middle of its sweep while it is not all the way in.
function wiperAt(w: Wiper, t: number) {
  const reach = reachOf(w, t);
  if (reach <= 0) return null;
  const a = baseAngle(w, t);
  const spin = (baseAngle(w, t + 0.02) - baseAngle(w, t - 0.02)) / 0.04;
  const v = strokeAge(w, t);
  const drag = -clamp(spin * 0.16, -1.1, 1.1);
  const hook = w.side * (2.3 - 1.1 * Math.min(1, Math.abs(spin) / 3));
  const heading = (u: number) =>
    a + drag * u ** 1.6 + hook * smooth(0.62, 1, u) + 0.22 * u * Math.sin(2 * Math.PI * (u * 1.4 - v * 1.1 + w.seed));
  const pose: Pose = { arms: { [w.arm]: { heading } } };
  const [rx, ry] = RIG.root(w.arm);
  const scale = (w.reach * H) / RIG.length(w.arm);
  const mid = (((w.sweep[0] + w.sweep[1]) / 2) * Math.PI) / 180;
  const back = scale * 70 + (1 - reach) * w.reach * H * 0.9;
  const bx = lx(bounds.x + w.from[0] * bounds.w) - Math.cos(mid) * back;
  const by = ly(bounds.y + w.from[1] * bounds.h) - Math.sin(mid) * back;
  return { pose, m: new DOMMatrix().translate(bx, by).scale(scale).translate(-rx, -ry) };
}

function wiper(g: Context, w: Wiper, t: number, look: Look) {
  const p = wiperAt(w, t);
  if (!p) return;
  g.save();
  g.transform(p.m.a, p.m.b, p.m.c, p.m.d, p.m.e, p.m.f);
  RIG.draw(g, p.pose, { ...look, only: w.arm });
  g.restore();
}

// Every arm's root and tip in device pixels over its whole stroke, one sample per SAMPLE seconds from its start:
// sweepAt(w, t) is the sample at or before t, or null outside the stroke.
interface Sample {
  s: number;
  root: DOMPoint;
  tip: DOMPoint;
  reach: number;
}
const SAMPLE = 1 / 120;
function sweep() {
  return new Map(
    WIPERS.map((w) => {
      const out: (Sample | null)[] = [];
      for (let s = AT.back + w.at; s <= AT.back + w.at + w.lasts + IN + OUT; s += SAMPLE) {
        const p = wiperAt(w, s);
        if (!p) {
          out.push(null);
          continue;
        }
        const root = p.m.transformPoint(new DOMPoint(...RIG.root(w.arm)));
        const tip = p.m.transformPoint(new DOMPoint(...RIG.tip(p.pose, w.arm)));
        out.push({ s, root, tip, reach: reachOf(w, s) });
      }
      return [w, out];
    }),
  );
}
const sweepAt = (w: Wiper, t: number) => sweeps.get(w)![Math.floor((t - AT.back - w.at) / SAMPLE)] ?? null;

// Everything the arms have swept since the finale started, kept in `wiped` and brought up to t one 60th of a second at
// a time: each step fills the wedge from the arm's root, out to its tip, between where the tip was a step ago and where
// it is now, and the arm's own shape as it stands at t.
let wipedTo: number;
const WIPE_STEP = 1 / 60;
const pointed = new Map<Wiper, DOMPoint>();
function stamp(t: number) {
  const g = wiped.getContext("2d")!;
  g.fillStyle = g.strokeStyle = BLACK;
  g.lineWidth = 3 * DPR;
  g.lineJoin = "round";
  for (let s = wipedTo + WIPE_STEP; s <= t; s += WIPE_STEP) {
    for (const w of WIPERS) {
      const at = sweepAt(w, s);
      if (!at || at.reach < 0.6) {
        pointed.delete(w);
        continue;
      }
      const before = pointed.get(w) ?? at.tip;
      g.beginPath();
      g.moveTo(at.root.x, at.root.y);
      g.lineTo(before.x, before.y);
      g.lineTo(at.tip.x, at.tip.y);
      g.closePath();
      g.fill();
      g.stroke();
      pointed.set(w, at.tip);
    }
    wipedTo = s;
  }
  for (const w of WIPERS) wiper(g, w, t, { solid: BLACK });
}

// Ink flung off each tip where it whips fastest: squares leave the tip with its velocity and fall.
function spray(g: Context, t: number) {
  const p = new Path2D();
  const step = 4;
  for (const w of WIPERS) {
    const path = sweeps.get(w)!;
    for (let k = 0; k + step < path.length; k += step) {
      const a = path[k];
      const b = path[k + step];
      if (!a || !b || a.s > t) continue;
      const age = t - a.s;
      if (age > 0.8) continue;
      const dt = b.s - a.s;
      const vx = (b.tip.x - a.tip.x) / dt;
      const vy = (b.tip.y - a.tip.y) / dt;
      if (Math.hypot(vx, vy) < H * 1.8) continue;
      for (let j = 0; j < 2; j++) {
        const i = a.s * 1000 + j * 7 + w.seed * 13;
        const f = 0.45 + 0.35 * hash(i);
        const side = (hash(i + 3) - 0.5) * 0.4;
        const x = a.tip.x + (vx * f - vy * side) * age;
        const y = a.tip.y + (vy * f + vx * side) * age + H * 2.2 * age * age;
        const s = T * (0.18 + 0.28 * hash(i + 5)) * (1 - age / 0.8) ** 0.5;
        p.rect(x - s / 2, y - s / 2, s, s);
      }
    }
  }
  g.fillStyle = C.navy;
  g.fill(p);
}

// The wet streaks: for each arm, the arcs traced by four points near its tip, out from its root, laid down while it
// sweeps as thin broken lines, a pale line over a navy one so they read on any desktop. Each stretch dries a while
// after it was laid, later toward the end of the show, so the last streak dries as the song ends. A trail is one
// stretch: a run of [x, y, when laid] points, its opacity and how long after its start it dries.
interface Trail {
  points: [number, number, number][];
  alpha: number;
  dry: number;
}
function streaked() {
  const out: Trail[] = [];
  WIPERS.forEach((w) =>
    [0.97, 0.91, 0.84, 0.76].forEach((f, k) => {
      let run: Trail | null = null;
      sweeps.get(w)!.forEach((at, i) => {
        if (!at || at.reach < 0.8 || i % 2) return;
        const n = i >> 1;
        if (n % 6 === 0) {
          const broken = hash(Math.floor(n / 6) * 7.3 + k + w.seed * 11) < 0.3;
          const dry = lerp(0.9, AT.end - 0.15 - at.s, 0.55) + 0.3 * hash(n + k * 5 + w.seed);
          run = broken ? null : { points: [], alpha: 0.4 - 0.07 * k, dry };
          if (run) out.push(run);
        }
        run?.points.push([lerp(at.root.x, at.tip.x, f), lerp(at.root.y, at.tip.y, f), at.s]);
      });
    }),
  );
  return out.filter((run) => run.points.length > 1);
}

function streaks(g: Context, t: number) {
  const wet = byOpacity(20);
  for (const run of trails) {
    const age = t - run.points[0]![2];
    if (age < 0 || age > run.dry) continue;
    const path = wet.at(run.alpha * (1 - smooth(run.dry - 0.35, run.dry, age)));
    if (!path) continue;
    path.moveTo(run.points[0]![0], run.points[0]![1]);
    for (const [x, y, s] of run.points) {
      if (s > t) break;
      path.lineTo(x, y);
    }
  }
  for (const [stroke, width, share] of [
    [C.navy, 3, 0.5],
    [C.white, 1.5, 1],
  ] as const) {
    g.strokeStyle = stroke;
    g.lineWidth = width * DPR;
    for (const [level, path] of wet.paths) {
      g.globalAlpha = level * share;
      g.stroke(path);
    }
  }
  g.globalAlpha = 1;
}
