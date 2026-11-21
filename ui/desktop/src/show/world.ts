// The octopus's world: it wakes on its paper and swims, its arms kicking on the beat, opens its eye at you, throws its
// arms out to the edges on bar 7, then charges at you twice on the 8ths with its arms tucked, slams flat against the
// inside of your screen with its suckers splayed on the glass, and squirts ink until everything is dark; then inside
// its pupil, dark.

import { inOut, lerp, outBack, outCubic, smooth } from "../octopus/ease";
import type { Pose } from "../octopus/rig";
import { EX, EY, lookFrom, octopus, scaleFor, swim, whip } from "./creature";
import { C, disc, H, jitter, ring, T, W, type Context } from "./display";
import { AT, BEAT, HITS, hitsUpTo, lastHit, pulse } from "./music";
import { paper, WORLD_SIZE } from "./stills";

/** How long the squirt takes to cover the screen before the dark. */
export const SQUIRT = 0.2;
const CHARGE = [1, 1.18, 1.36, 1.55];
const SLAM = 2;

function lidAt(t: number) {
  if (t < AT.open) return 1;
  if (t < AT.open + 0.18) return 1 - outCubic((t - AT.open) / 0.18);
  const b = t - AT.blink;
  if (b < 0) return 0;
  if (b < 0.07) return (b / 0.07) ** 2;
  if (b < 0.13) return 1;
  return 1 - outCubic((b - 0.13) / 0.12);
}

export function world(g: Context, t: number) {
  g.drawImage(paper, 0, 0);
  const base = scaleFor(WORLD_SIZE);
  let pose: Pose;
  let scale = base;
  let squash = 0;
  let focus: [number, number] = [0, 0];
  if (t < AT.reach) pose = swim(t, t < AT.open ? 0.3 : 0.45, 0.9, whip(t, 0.25));
  else if (t < AT.lunge) {
    const e = outCubic((t - AT.reach) / BEAT);
    pose = swim(t, 0.35, 1.1, (i) => {
      const w = whip(t, 0.6)(i);
      return { curl: -1.3 * e + w.curl, swing: w.swing, stretch: 0.3 * e };
    });
  } else {
    const step = Math.min(SLAM, Math.floor((t - AT.lunge) / (BEAT / 2)));
    const age = t - AT.lunge - (step * BEAT) / 2;
    const hit = outBack(age / 0.08);
    const slammed = step === SLAM ? outCubic(age / 0.06) : 0;
    const slid = step === SLAM ? age : 0;
    pose = swim(t, 0.12 * (1 - slammed), 1.1, () => ({
      curl: lerp(1.3, -1.3, slammed) + 0.5 * slid,
      stretch: lerp(-0.15, 0.3, slammed) + 0.4 * slid,
    }));
    scale = base * lerp(CHARGE[step]!, CHARGE[step + 1]!, hit);
    squash = step === SLAM ? 0.05 + 0.1 * Math.exp(-age / 0.07) : 0;
    focus = [EX * 0.5, EY * 0.5];
  }
  const x = W / 2;
  const y = H / 2;
  const ex = x + (EX - focus[0]) * scale;
  const ey = y + (EY - focus[1]) * scale;
  if (t >= AT.reach && t < AT.lunge) {
    for (const h of hitsUpTo("kick", AT.reach, t)) {
      const age = t - h.t;
      if (age < 1.2) {
        const band = T * 0.5 * (1 - age / 1.2) + T * 0.1;
        ring(g, ex, ey, H * 0.1 + age * H * 1.3, band, T * 0.25, h.i % 2 ? C.deep : C.navy);
      }
    }
  }
  const landed = t - (AT.lunge + (SLAM * BEAT) / 2);
  g.save();
  g.translate(x, y);
  jitter(g, t, landed > 0 ? T * 0.6 * Math.exp(-landed / 0.08) : 0);
  g.scale(1 + squash, 1 - squash * 0.8);
  g.translate(-x, -y);
  octopus(g, {
    x,
    y,
    scale,
    focus,
    pose,
    lid: lidAt(t),
    look: lookFrom(ex, ey),
    sparkle: pulse("hat", t, 0.07),
    seed: lastHit(HITS.hat, t),
    shadow: t >= AT.reach ? 14 : 0,
  });
  g.restore();
  if (landed > 0) glint(g, landed);
  const squirt = (t - (AT.dark - SQUIRT)) / SQUIRT;
  if (squirt > 0) {
    const r = Math.hypot(W, H) * 0.6 * outCubic(squirt);
    disc(g, x, y, r + T * 0.8, T / 2, C.gold);
    disc(g, x, y, r, T / 2, C.navy);
  }
}

// A white streak of reflection sliding across the glass the octopus just hit.
function glint(g: Context, age: number) {
  const a = 0.45 * (1 - smooth(0.3, 0.45, age));
  const at = lerp(0, 1.2, inOut(age / 0.45)) * (W + H);
  if (a < 0.02) return;
  g.save();
  g.globalAlpha = a;
  g.fillStyle = C.white;
  for (const [off, wide] of [
    [0, T * 3],
    [T * 4, T * 0.8],
  ] as const) {
    g.beginPath();
    g.moveTo(at - off, 0);
    g.lineTo(at - off + wide, 0);
    g.lineTo(at - off + wide - H, H);
    g.lineTo(at - off - H, H);
    g.fill();
  }
  g.restore();
}

// Inside the pupil: dark, and a cursor blinks twice in the gap before the drop.
export function dark(g: Context, t: number) {
  g.fillStyle = C.navy;
  g.fillRect(0, 0, W, H);
  const b = t - (AT.drop - BEAT);
  if ((b >= 0 && b < 0.15) || (b >= 0.25 && b < 0.4)) {
    g.fillStyle = C.paper;
    g.fillRect(W / 2 - T * 0.3, H / 2 - T * 0.5, T * 0.6, T * 1.0);
  }
}
