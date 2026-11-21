// From the start to the turn: the live desktop, infected before the music, then bare after the blackout, and something
// knocks from behind it on the kicks, harder each bar; then the turn, where every tile turns over to the octopus's
// world.

import { clamp, hash, inOut, lerp, outCubic, smooth } from "../octopus/ease";
import { bounds, C, COLS, cut, lx, ly, mouse, ROWS, T, TH, W, H, type Context } from "./display";
import { infect } from "./infection";
import { AT, bar, BEAT, GONE, HITS, hitsBetween, lastHit, pulse, SHOW, type Hit } from "./music";
import { bare } from "./stills";
import { eachTile, shapeOf, tileCenter, tileGlobal, tiles, type Shape } from "./tiles";
import { world } from "./world";

/** The first knocks land beside the pointer, twice as hard; the rest land anywhere. */
const NEAR = 4;
// How many tiles knock on the music's kick i: one hard knock beside the cursor, a beat of nothing, then more on every
// kick.
const knocks = (i: number) => [1, 0, 1, 2][i] ?? Math.min(60, Math.round(2 * 1.4 ** (i - 2)));
const FIRST_KNOCK = lastHit(HITS.kick, SHOW.musicIn - 1e-6) + 1;

export function haunt(g: Context, t: number) {
  g.clearRect(0, 0, W, H);
  if (t >= GONE) {
    const grout = 0.13 * smooth(bar(4), AT.turn, t) + 0.05 * pulse("kick", t, 0.1) * (t > bar(4) ? 1 : 0);
    const quake = T * 0.12 * smooth(bar(4) + 2 * BEAT, AT.turn, t);
    if (grout > 0.003) {
      g.fillStyle = C.navy;
      g.fillRect(0, 0, W, H);
      const q = Math.floor(t * 30);
      const shapes: Shape[] = [];
      eachTile((col, row, i) => {
        const [x, y] = tileCenter(col, row);
        shapes.push(
          shapeOf(col, row, {
            x: x + quake * (hash(i * 3 + q) - 0.5),
            y: y + quake * (hash(i * 5 + q + 7) - 0.5),
            s: 1 - grout,
          }),
        );
      });
      tiles(g, shapes);
    } else g.drawImage(bare, 0, 0);
  }
  for (const h of hitsBetween("kick", SHOW.musicIn, AT.turn)) {
    const age = t - h.t;
    if (age < 0 || age > 0.5) continue;
    if (h.t >= bar(2)) slide(g, h, age);
    knock(g, h, age);
  }
  if (t < SHOW.musicIn) infect(g, t);
}

// The tiles kick `h` knocks, `age` seconds after it: each lifts off its navy hole, and from bar 3 some peek over.
function knock(g: Context, h: Hit, age: number) {
  const n = h.i - FIRST_KNOCK;
  const hard = n < NEAR ? 2 : 1;
  const holes = new Path2D();
  const shapes: Shape[] = [];
  for (let j = 0; j < knocks(n); j++) {
    const near = [
      clamp(Math.floor(lx(mouse.x) / T) + Math.round((hash(h.i * 7 + j) - 0.5) * 6), 0, COLS - 1),
      clamp(Math.floor(ly(mouse.y) / TH) + Math.round((hash(h.i * 9 + j) - 0.5) * 4), 0, ROWS - 1),
    ] as const;
    const i = n < NEAR ? near[1] * COLS + near[0] : Math.floor(hash(h.i * 1000 + j) * COLS * ROWS);
    const col = i % COLS;
    const row = Math.floor(i / COLS);
    const [x, y] = tileCenter(col, row);
    const lifted = Math.exp(-age / (0.09 * hard));
    const peek = h.t >= bar(3) && hash(i + h.i) > 0.45;
    const th = peek ? Math.PI * (age < 0.12 ? inOut(age / 0.12) : age < 0.3 ? 1 : 1 - inOut((age - 0.3) / 0.15)) : 0;
    if (!peek && lifted < 0.02) continue;
    holes.rect(x - T / 2, y - TH / 2, T, TH);
    shapes.push(
      shapeOf(col, row, {
        x: x - lifted * T * 0.06 * hard,
        y: y - lifted * T * 0.1 * hard,
        s: 1 + 0.16 * hard * lifted,
        th,
      }),
    );
  }
  g.fillStyle = C.navy;
  g.fill(holes);
  tiles(g, shapes);
}

// A band of rows slips sideways and snaps back.
function slide(g: Context, h: Hit, age: number) {
  const r0 = Math.floor(hash(h.i * 31) * ROWS);
  const rows = 1 + (h.i % 3);
  const out = age < 0.1 ? 1 : 1 - outCubic((age - 0.1) / 0.12);
  if (out <= 0) return;
  const dx = (hash(h.i * 17) > 0.5 ? 1 : -1) * T * (1 + Math.floor(hash(h.i * 13) * 5)) * out;
  g.fillStyle = C.navy;
  g.fillRect(0, r0 * TH, W, rows * TH);
  for (const k of [-1, 0, 1]) cut(g, bare, 0, r0 * TH, W, rows * TH, dx + k * W, r0 * TH, W, rows * TH);
}

/** The turn's wave reaches the farthest tile SPREAD after it starts, give or take JITTER, and each tile turns over in
 * TURNING; by TURNED every tile shows the world. */
const SPREAD = 0.5;
const JITTER = 0.12;
const TURNING = 0.26;
export const TURNED = AT.turn + SPREAD + JITTER + TURNING;

// The turn: from the pointer outward, across every display, each tile turns over to its square of the octopus's
// world, which is already alive underneath. The world goes down whole, the tiles still waiting go over it as one
// clipped picture on navy, and only the turning ones are drawn alone, their backs cut from the world's still picture.
let waveOrigin: { x: number; y: number } | null = null;
export function turn(g: Context, t: number) {
  waveOrigin ??= { ...mouse };
  const origin = waveOrigin;
  const reach = Math.hypot(bounds.w, bounds.h);
  const q = Math.floor(t * 30);
  // The tiles not yet over sit on navy, a row's run of them as one rectangle.
  const navy = new Path2D();
  const waiting = new Path2D();
  const turning: Shape[] = [];
  let run: number | null = null;
  eachTile((col, row, i) => {
    const [gx, gy] = tileGlobal(col, row);
    const [x, y] = tileCenter(col, row);
    const start = AT.turn + (SPREAD * Math.hypot(gx - origin.x, gy - origin.y)) / reach + JITTER * hash(i);
    const u = clamp((t - start) / TURNING);
    if (u < 1) run ??= col;
    if (run !== null && (u >= 1 || col === COLS - 1)) {
      navy.rect(run * T, row * TH, ((u >= 1 ? col : col + 1) - run) * T, TH);
      run = null;
    }
    if (u <= 0) waiting.rect(x - T * 0.435, y - TH * 0.435, T * 0.87, TH * 0.87);
    else if (u < 1)
      turning.push(
        shapeOf(col, row, { x, y, th: Math.PI * inOut(u), s: lerp(0.87, 1, u), across: hash(i * 11) > 0.5 }),
      );
  });
  world(g, t);
  g.fillStyle = C.navy;
  g.fill(navy);
  g.save();
  g.clip(waiting);
  g.drawImage(bare, T * 0.1 * (hash(q) - 0.5), T * 0.1 * (hash(q + 3) - 0.5));
  g.restore();
  turning.sort((a, b) => a.lift - b.lift);
  tiles(g, turning);
}
