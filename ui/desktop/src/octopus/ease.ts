/** The film's easing: every move eases in and out of rest along one of these curves. */

export const clamp = (value: number, low: number, high: number) => Math.min(high, Math.max(low, value));

/** 0 before `from`, 1 after `to`, and a smooth S between, so neighbouring squares always move by nearly as much. */
export const smooth = (from: number, to: number, value: number) => {
  const x = clamp((value - from) / (to - from), 0, 1);
  return x * x * (3 - 2 * x);
};

/** A CSS cubic-bezier as a function of progress from 0 to 1. */
export function bezier(x1: number, y1: number, x2: number, y2: number) {
  const curve = (a: number, b: number, s: number) => 3 * a * s * (1 - s) ** 2 + 3 * b * s * s * (1 - s) + s ** 3;
  return (x: number) => {
    if (x <= 0) return 0;
    if (x >= 1) return 1;
    let [low, high, s] = [0, 1, x];
    for (let i = 0; i < 24; i++) {
      if (curve(x1, x2, s) < x) low = s;
      else high = s;
      s = (low + high) / 2;
    }
    return curve(y1, y2, s);
  };
}

/** Leaves fast and lands softly. */
export const out = bezier(0.16, 1, 0.3, 1);
/** Starts and lands softly; the sidebar's pull runs on the same curve as `--ease-pull`. */
export const inOut = bezier(0.65, 0, 0.35, 1);

export const hash = (n: number) => {
  const s = Math.sin(n * 127.1 + 311.7) * 43758.5453;
  return s - Math.floor(s);
};
