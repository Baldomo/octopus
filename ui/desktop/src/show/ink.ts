// Ink: the tour's navy runs off the display, and the octopus over the desktop sprays ink on every kick and loud clap,
// in blots of squares that run in drips; on bar 16 the blots land on top of it too. The ink is rimmed in saffron so it
// reads on any desktop: every rim goes down before any ink, so a rim only shows round the outside of the whole stain.

import { clamp, hash, lerp, outCubic, smooth } from "../octopus/ease";
import { app, outline, windowBox } from "./app";
import { EX, EY, lookFrom, octopus, scaleFor, swim, whip } from "./creature";
import { C, H, layer, picture, shake, T, W, type Context } from "./display";
import { AT, BEAT, HITS, hitsBetween, lastHit, pulse } from "./music";

/** The ink's grid: three squares to a tile. */
const pitch = () => T / 3;

// The tour's navy turns to ink: column by column of squares it runs down off the display, each column led by a saffron
// rim as the ink's blots are, from the window outward, over a beat and a half. The octopus comes up from below beneath
// it, so the running ink uncovers it with the desktop; the live window stays where it is throughout.
const MELT = 1.5 * BEAT;
function melt(g: Context, age: number) {
  if (age >= MELT) return;
  const win = windowBox(app().frame);
  const q = pitch();
  const middle = win.x + win.w / 2;
  const rims = new Path2D();
  const columns = new Path2D();
  for (let x = 0, k = 0; x < W; x += q, k++) {
    const away = Math.abs(x + q / 2 - middle) / W;
    const start = (0.35 * away + 0.15 * hash(k * 13 + 1)) * MELT;
    const fall = clamp((age - start) / (MELT * 0.55)) ** 2 * H * 1.15;
    rims.rect(x, fall - q, q + 0.5, q);
    columns.rect(x, fall, q + 0.5, H - fall);
  }
  // Everything but the window, which stays live through its hole.
  const around = new Path2D();
  around.rect(0, 0, W, H);
  outline(around, win);
  g.save();
  g.clip(around, "evenodd");
  g.fillStyle = C.paper;
  g.fill(rims);
  g.fillStyle = C.navy;
  g.fill(columns);
  g.restore();
}

/** Where a blot lands and when: its place and size as shares of the display. */
interface Landing {
  t: number;
  j: number;
  x: number;
  y: number;
  r: number;
}
/** A blot as baked for this display: its landing, and its rim and ink pictures on the display's ink grid. */
type Splat = Landing & ReturnType<typeof bake>;
/** A blot picture's pixels across one of its squares: it is drawn up to the display's squares unsmoothed, so a few
 * pixels a square keep it sharp at a fraction of the memory the display's own pixels would take. */
const BLOT_PIXELS = 10;
const LANDINGS: Landing[] = [
  ...hitsBetween("kick", AT.ink, AT.back),
  ...hitsBetween("clap", AT.ink, AT.back).filter((h) => h.v > 0.85),
]
  .sort((a, b) => a.t - b.t)
  .map((h, j) => ({
    t: h.t,
    j,
    x: 0.08 + 0.84 * hash(j * 11 + 3),
    y: 0.08 + 0.8 * hash(j * 17 + 5),
    r: (0.13 + 0.1 * hash(j * 23)) * (1 + j * 0.09),
  }));

// A blot at full size, drawn once: its rim and its ink as two pictures on the display's ink grid.
function bake(s: Landing) {
  const q = pitch();
  const cx = s.x * W;
  const cy = s.y * H;
  const R = s.r * H;
  const reach = R * 1.9 + 2 * q;
  const rays = Array.from({ length: 7 }, (_, k) => [hash(s.j * 31 + k) * Math.PI * 2, 1.25 + hash(s.j * 37 + k) * 0.6]);
  const radius = (a: number) => {
    let r = 0.78 + 0.22 * Math.sin(a * 3 + s.j) * Math.sin(a * 5 + s.j * 2);
    for (const [ra, len] of rays) {
      const d = Math.abs(((a - ra! + Math.PI * 3) % (Math.PI * 2)) - Math.PI);
      if (d < 0.12) r = Math.max(r, len! * (1 - d / 0.12));
    }
    return r * R;
  };
  const x0 = Math.floor((cx - reach) / q) * q;
  const y0 = Math.floor((cy - reach) / q) * q;
  const squares = Math.ceil((2 * reach) / q + 1);
  const n = BLOT_PIXELS;
  // How far inside the blot's edge each square's centre sits, in device pixels.
  const cells: [number, number, number][] = [];
  for (let row = 0; row < squares; row++)
    for (let col = 0; col < squares; col++) {
      const dx = x0 + (col + 0.5) * q - cx;
      const dy = y0 + (row + 0.5) * q - cy;
      cells.push([col, row, radius(Math.atan2(dy, dx)) - Math.hypot(dx, dy)]);
    }
  const rim = picture(
    (g) => {
      g.fillStyle = C.paper;
      for (const [col, row, inside] of cells) if (inside > -q * 1.5) g.fillRect(col * n, row * n, n, n);
    },
    squares * n,
    squares * n,
  );
  const ink = picture(
    (g) => {
      g.fillStyle = C.navy;
      for (const [col, row, inside] of cells) {
        if (inside <= -q * 0.5) continue;
        const side = Math.max(1, Math.round(n * Math.min(1, 0.3 + inside / (q * 2.5))));
        const gap = Math.floor((n - side) / 2);
        g.fillRect(col * n + gap, row * n + gap, side, side);
      }
    },
    squares * n,
    squares * n,
  );
  return {
    rim,
    ink,
    x0,
    y0,
    cx,
    cy,
    q,
    R,
    size: squares * q,
  };
}

// One pass of a blot `age` seconds after it landed, growing in over 0.14 s, with its drips running down: its rim, or
// its ink.
function blot(g: Context, s: Splat, age: number, pass: "rim" | "ink") {
  const grow = outCubic(age / 0.14);
  const edge = pass === "rim" ? s.q : 0;
  g.fillStyle = pass === "rim" ? C.paper : C.navy;
  g.imageSmoothingEnabled = false;
  g.drawImage(s[pass], s.cx + (s.x0 - s.cx) * grow, s.cy + (s.y0 - s.cy) * grow, s.size * grow, s.size * grow);
  g.imageSmoothingEnabled = true;
  for (let k = 0; k < 4; k++) {
    const x = Math.floor((s.cx + (hash(s.j * 41 + k) - 0.5) * s.R * 1.2) / s.q) * s.q;
    const top = s.cy + s.R * 0.4;
    const len = H * 0.45 * smooth(0, 1.6, age - 0.1) * (0.4 + hash(s.j * 43 + k));
    const width = s.q * (1 + (k % 2));
    g.fillRect(x - edge, top, width + 2 * edge, len + edge);
    g.fillRect(x - s.q / 2 - edge, top + len - s.q - edge, width + s.q + 2 * edge, s.q * 1.5 + 2 * edge);
  }
}

// A blot stops changing once its drips have run, DRIED after it lands; from then it lives in its stain, a layer the
// blots of one list are painted into once, in the order they land.
const DRIED = 1.7;
interface Stain {
  list: Splat[];
  layer: OffscreenCanvas;
  painted: number;
}
let under: Stain;
let over: Stain;

/** Bakes every blot, paints `last`, the ink as it stands when the finale starts, and opens the stains the show paints
 * into. Hands back the pictures to be warmed, and `last`. */
export function prepareInk() {
  const splats: Splat[] = LANDINGS.map((s) => ({ ...s, ...bake(s) }));
  const open = () => {
    under = { list: splats.filter((s) => s.t < AT.drown), layer: layer(), painted: 0 };
    over = { list: splats.filter((s) => s.t >= AT.drown), layer: layer(), painted: 0 };
  };
  open();
  const last = picture((g) => ink(g, AT.back));
  open();
  const pictures: CanvasImageSource[] = [last, under.layer, over.layer];
  return { last, pictures: [...pictures, ...splats.flatMap((s) => [s.rim, s.ink])] };
}

// The blots of a list that have landed by t: every rim lies under every ink. A dried blot's rim goes behind its stain
// and its ink over it, so the stain keeps that order, and the blots still drying go round it.
function splats(g: Context, t: number, stained: Stain) {
  const stain = stained.layer.getContext("2d")!;
  for (let s = stained.list[stained.painted]; s && s.t + DRIED <= t; s = stained.list[++stained.painted]) {
    stain.globalCompositeOperation = "destination-over";
    blot(stain, s, DRIED, "rim");
    stain.globalCompositeOperation = "source-over";
    blot(stain, s, DRIED, "ink");
  }
  const drying = stained.list.slice(stained.painted).filter((s) => s.t <= t);
  for (const s of drying) blot(g, s, t - s.t, "rim");
  g.drawImage(stained.layer, 0, 0);
  for (const s of drying) blot(g, s, t - s.t, "ink");
}

export function ink(g: Context, t: number) {
  g.clearRect(0, 0, W, H);
  g.save();
  shake(g, t, 0.4);
  splats(g, t, under);
  const scale = scaleFor(0.85) * (1 + 0.04 * pulse("kick", t, 0.1));
  const rise = smooth(AT.ink, AT.ink + 0.3, t);
  const y = lerp(H * 0.95, H / 2, outCubic((t - AT.ink) / MELT));
  octopus(g, {
    x: W / 2,
    y,
    scale,
    pose: swim(t, 0.6 * rise, 1.8, whip(t, 1.1 * rise)),
    look: lookFrom(W / 2 + EX * scale, y + EY * scale),
    sparkle: pulse("hat", t, 0.06),
    seed: lastHit(HITS.hat, t),
    shadow: 16,
  });
  splats(g, t, over);
  g.restore();
  melt(g, t - AT.ink);
}
