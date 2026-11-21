// The show's octopus: the rig drawn at a place and size on the display, swimming to the music and looking at the
// pointer.

import { ACROSS, rig, type ArmPose, type Look, type Pose } from "../octopus/rig";
import { H, lx, ly, mouse, type Context } from "./display";
import { pulse } from "./music";

export const RIG = rig();
/** The eye's centre at rest, in rig units. */
export const [EX, EY] = RIG.eye;

/** The octopus filling `share` of the display's height. */
export const scaleFor = (share: number) => (H * share) / ACROSS;

interface Drawn extends Omit<Look, "pupil"> {
  x: number;
  y: number;
  scale: number;
  /** The rig point drawn at (x, y). */
  focus?: [number, number];
  pose?: Pose;
  look?: [number, number];
}
export function octopus(g: Context, { x, y, scale, focus = [0, 0], pose = {}, look = [0, 0], ...rest }: Drawn) {
  g.save();
  g.translate(x, y);
  g.scale(scale, scale);
  g.translate(-focus[0], -focus[1]);
  RIG.draw(g, pose, { pupil: look, ...rest });
  g.restore();
}

// Every arm swimming at once: a travelling wave of `amp` radians at `speed`, plus whatever arm(i) adds for arm i, and
// the body swelling on the kick.
export function swim(t: number, amp: number, speed: number, arm: (i: number) => ArmPose): Pose {
  const arms = Object.fromEntries(RIG.arms.map((name, i) => [name, { wave: amp, phase: t * speed, ...arm(i) }]));
  return { arms, breathe: 0.03 * pulse("kick", t, 0.12) };
}

// Every arm whips on the kick, odd arms one way and even arms the other.
export const whip = (t: number, amount: number) => (i: number) => {
  const w = pulse("kick", t, 0.15) * amount;
  return { swing: (i % 2 ? 1 : -1) * w, curl: (i % 2 ? -1 : 1) * w };
};

// Which way the pupil looks from the eye (at device px) toward the pointer, -1, 0 or 1 each way: straight ahead while
// the pointer is near the eye.
export function lookFrom(ex: number, ey: number): [number, number] {
  const dx = lx(mouse.x) - ex;
  const dy = ly(mouse.y) - ey;
  const n = Math.hypot(dx, dy);
  if (n < 1) return [0, 0];
  const f = Math.min(1, n / (H * 0.3));
  return [Math.sign(Math.round((dx / n) * f * 2.4)), Math.sign(Math.round((dy / n) * f * 1.4))];
}
