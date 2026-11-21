// The song's timing: its bars and beats, the moments the show's scenes start on, and the hits of its drums, read from
// the song's analysis in show.json.

import SHOW from "./show.json";

export { SHOW };

export interface Hit {
  t: number;
  v: number;
  i: number;
}

const T0 = SHOW.first;
const STEP = SHOW.step;
export const BEAT = 0.5;
const BAR = 2;
export const bar = (n: number) => T0 + BAR * (n - 1);
export const AT = {
  turn: bar(4) + 3 * BEAT,
  open: bar(6),
  blink: bar(6) + 3 * BEAT,
  reach: bar(7),
  lunge: bar(8),
  dark: bar(8) + 2 * BEAT,
  drop: bar(9),
  land: bar(10),
  ink: bar(15),
  drown: bar(16),
  back: bar(17),
  end: SHOW.duration,
};
export const INFECTION = SHOW.infection;
const blackout = INFECTION.find((e) => e.look === "blackout")!;
/** From the blackout's end until the live desktop returns, the display shows the bare wallpaper. */
export const GONE = blackout.t + blackout.lasts;

type Band = "kick" | "clap" | "hat";
// Hits on the 16th-note grid: a 16th louder than both neighbours and over the band's threshold.
function hitsOf(name: Band, threshold: number): Hit[] {
  const g = SHOW.grid[name];
  const out: Hit[] = [];
  g.forEach((v, k) => {
    if (v >= threshold && v >= (g[k - 1] ?? 0) && v >= (g[k + 1] ?? 0))
      out.push({ t: T0 + k * STEP, v, i: out.length });
  });
  return out;
}
export const HITS: Record<Band, Hit[]> = {
  kick: hitsOf("kick", 0.7),
  clap: hitsOf("clap", 0.6),
  hat: hitsOf("hat", 0.55),
};

/** The index of the last hit at or before t, or -1. */
export function lastHit(list: Hit[], t: number) {
  let lo = 0;
  let hi = list.length - 1;
  let found = -1;
  while (lo <= hi) {
    const m = (lo + hi) >> 1;
    if (list[m]!.t <= t) {
      found = m;
      lo = m + 1;
    } else hi = m - 1;
  }
  return found;
}

/** The last hit's strength, decaying over `decay` seconds since it landed. */
export const pulse = (name: Band, t: number, decay: number) => {
  const hit = HITS[name][lastHit(HITS[name], t)];
  return hit ? hit.v * Math.exp(-(t - hit.t) / decay) : 0;
};

/** The hits from `from` up to but not including `to`. */
export const hitsBetween = (name: Band, from: number, to: number) =>
  HITS[name].slice(lastHit(HITS[name], from - 1e-6) + 1, lastHit(HITS[name], to - 1e-6) + 1);

/** The hits from `from` up to and including t. */
export const hitsUpTo = (name: Band, from: number, t: number) => hitsBetween(name, from, t + 1e-6);
