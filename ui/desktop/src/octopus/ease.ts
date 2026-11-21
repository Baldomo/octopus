/** The film's curves and the numbers every frame of it is drawn from: clamping, blending, easing and a fixed hash. */
export const clamp = (value: number, low = 0, high = 1) => Math.min(high, Math.max(low, value));

/** A fixed pseudo-random number in [0, 1) for each `n`, so a frame drawn twice comes out the same. */
export const hash = (n: number) => {
  const s = Math.sin(n * 127.1 + 311.7) * 43758.5453;
  return s - Math.floor(s);
};

/** 0 before `from`, 1 after `to`, and a smooth S between, so neighbouring squares always move by nearly as much. */
export const smooth = (from: number, to: number, value: number) => {
  const x = clamp((value - from) / (to - from), 0, 1);
  return x * x * (3 - 2 * x);
};

export const lerp = (a: number, b: number, f: number) => a + (b - a) * f;

/** Leaves fast and slows to a stop. */
export const outCubic = (x: number) => 1 - (1 - clamp(x)) ** 3;

/** Overshoots its end and settles back onto it. */
export const outBack = (x: number) => {
  const c = 2.2;
  return 1 + (c + 1) * (clamp(x) - 1) ** 3 + c * (clamp(x) - 1) ** 2;
};

/** A CSS cubic-bezier as a function of progress from 0 to 1. */
function bezier(x1: number, y1: number, x2: number, y2: number) {
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
