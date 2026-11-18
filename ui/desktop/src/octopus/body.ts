/**
 * The film's octopus in world units: the logo quantised to squares, centred on the origin and 960 units across, and how
 * it moves at rest. One breath swells the mantle and runs travelling waves out along the arms (eight directions, each
 * its own period and phase, blended by angle) over a small swell that never stops; the eye stays rigid and blinks now
 * and then.
 */
import art from "./octopus.json";
import { clamp, hash, out, smooth } from "./ease";

export type Point = { x: number; y: number };
export type Rect = { x: number; y: number; w: number; h: number };

/** The inks every square takes one of, in `octopus.json`'s order; each is the CSS token `--octo-<ink>`. */
export const INKS = ["navy", "white", "peri", "lav", "deep", "gold"] as const;
export type Ink = number;
export const [NAVY, WHITE, PERI, LAV, DEEP, GOLD] = [0, 1, 2, 3, 4, 5];

export const PITCH = art.pitch;
/** Half the octopus's span, from its centre to its furthest square's edge. */
export const REACH = 480;

export type Cell = { x: number; y: number; s: number; c: Ink; ink: number; opening: boolean };
export const CELLS: Cell[] = (art.cells as [number, number, number, number, number, number][]).map(
  ([x, y, s, c, ink, opening]) => ({ x, y, s, c, ink, opening: opening === 1 }),
);
const openings = CELLS.filter((cell) => cell.opening).map((cell) => cell.y);
const LID = { top: Math.min(...openings) - art.eyePitch / 2, bottom: Math.max(...openings) + art.eyePitch / 2 };

export type ArmName = keyof typeof art.arms;
export const ARM_ART = art.arms as Record<
  ArmName,
  { root: [number, number]; heading: number; width: number; curl: [number, number][] }
>;

const TAU = Math.PI * 2;
const MANTLE = { x: 100, y: -200 };
const JUNCTION = { x: -30, y: 60 };
const EYE = { x: art.eye[0]!, y: art.eye[1]! };
/** One breath, in seconds, and how far it swells the mantle at its fullest. */
const BREATH = 4.8;
const SWELL = 0.014;
/** World units at the arms' reach: the wave that never stops, and what a full breath adds. */
const WAVE = { base: 7, breath: 12 };
const WAVELENGTH = 420;
/** World units a second that a breath's swell runs out along an arm. */
const SPEED = 2600;
/** How sharply the eight wave directions blend by angle. */
const BLEND = 2.2;

const WAVES = Array.from({ length: 8 }, (_, k) => {
  const a = (k / 8) * TAU;
  return { a, period: 2.5 + 1.5 * hash(k + 1), phase: TAU * hash(k + 11), nx: -Math.sin(a), ny: Math.cos(a) };
});

/** How full the breath is at t, from 0 to 1. */
const breath = (t: number) => (1 - Math.cos((TAU * t) / BREATH)) / 2;

/** The wave's displacement at (dx, dy) from the junction: nothing at the body, full from 560 units out. */
function wave(dx: number, dy: number, t: number): Point {
  const r = Math.hypot(dx, dy);
  const amp = (WAVE.base + WAVE.breath * breath(t - r / SPEED)) * smooth(110, 560, r) ** 1.2;
  if (amp <= 0) return { x: 0, y: 0 };
  const th = Math.atan2(dy, dx);
  let [wx, wy, sum] = [0, 0, 0];
  for (const w of WAVES) {
    const weight = Math.exp(BLEND * Math.cos(th - w.a));
    const v = Math.sin((TAU * t) / w.period + w.phase - (TAU * r) / WAVELENGTH);
    wx += weight * v * w.nx;
    wy += weight * v * w.ny;
    sum += weight;
  }
  return { x: (amp * wx) / sum, y: (amp * wy) / sum };
}

/** Where the rest-grid point (x, y) sits at time t, moving `live` of its full idle: rigid at the eye. */
export function poseAt(x: number, y: number, t: number, live: number): Point {
  const free = smooth(55, 120, Math.hypot(x - EYE.x, y - EYE.y)) * live;
  if (free <= 0) return { x, y };
  const swell = SWELL * breath(t) * (1 - smooth(180, 340, Math.hypot(x - MANTLE.x, y - MANTLE.y))) * free;
  const w = wave(x - JUNCTION.x, y - JUNCTION.y, t);
  return { x: x + (x - MANTLE.x) * swell * 0.6 + w.x * free, y: y + (y - MANTLE.y) * swell + w.y * free };
}

/** Each blink's start, a few seconds apart and never on a rhythm. */
const BLINK = { every: 5.5, close: 0.07, hold: 0.06, open: 0.12 };
/** How shut the eye is at t, from 0 to 1. */
function lid(t: number) {
  const n = Math.floor(t / BLINK.every);
  for (const k of [n, n - 1]) {
    const at = k * BLINK.every + 1 + 3 * hash(k * 7.31);
    if (t < at) continue;
    const s = t - at;
    if (s < BLINK.close) return (s / BLINK.close) ** 2;
    if (s < BLINK.close + BLINK.hold) return 1;
    if (s < BLINK.close + BLINK.hold + BLINK.open) return 1 - out((s - BLINK.close - BLINK.hold) / BLINK.open);
  }
  return 0;
}

/** Receives one square: its centre and side in world units, and its ink. */
export type Emit = (x: number, y: number, size: number, ink: Ink) => void;

/**
 * The octopus's squares at t that `shows`, moving `live` of its idle: the skin, then the logo's ink over it. A
 * tentacle stands in for the logo squares of its arm, which show only while it covers them.
 */
export function body(t: number, live: number, shows: (cell: Cell) => boolean, emit: Emit) {
  const shut = lid(t) * clamp(live, 0, 1);
  const line = LID.top + (LID.bottom - LID.top) * shut;
  for (const ink of [false, true]) {
    for (const cell of CELLS) {
      if (!shows(cell) || (ink && cell.ink === 0)) continue;
      let [size, c] = [ink ? cell.ink : cell.s, ink ? NAVY : cell.c];
      if (cell.opening && cell.y < line && shut > 0) {
        if (ink) continue;
        c = line - cell.y < art.eyePitch * 1.5 && shut > 0.2 ? NAVY : PERI;
      }
      const at = poseAt(cell.x, cell.y, t, live);
      emit(at.x, at.y, size, c);
    }
  }
}
