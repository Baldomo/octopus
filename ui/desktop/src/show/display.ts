// The display this page covers: its canvas, its size in device pixels and its grid of tiles, where it sits among every
// display, the pointer, and the drawing primitives every scene shares. Every scene and primitive draws into the context
// it is handed: the display's canvas, or a picture being painted.

import { inks } from "../octopus/rig";
import { hash } from "../octopus/ease";
import { pulse } from "./music";
import type { ShowScene } from "../generated";

export type Context = CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D;
export type Picture = ImageBitmap;
export interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

export const C = inks();
export const BLACK = "#000";

const canvas = document.querySelector<HTMLCanvasElement>("#screen")!;
export const screen = canvas.getContext("2d")!;

/** The display in device pixels, and its tiles: COLS across, each T wide and TH tall, in ROWS whole rows. */
export let W = 0;
export let H = 0;
export let DPR = 1;
export let T = 0;
export let TH = 0;
export const COLS = 32;
export let ROWS = 0;
/** The display, and the rect round every display, in global points. */
export let display: Box = { x: 0, y: 0, w: 0, h: 0 };
export let bounds: Box = { x: 0, y: 0, w: 0, h: 0 };
export let mouse = { x: 0, y: 0 };

export const follow = (pointer: { x: number; y: number }) => (mouse = pointer);

/** Sizes the canvas and the tiles to the scene's display. Tiles are a near-square that fits it in whole rows, so no
 * part-row lags the rest. */
export function fit(scene: ShowScene) {
  display = { x: scene.display.x, y: scene.display.y, w: scene.display.width, h: scene.display.height };
  bounds = { x: scene.bounds.x, y: scene.bounds.y, w: scene.bounds.width, h: scene.bounds.height };
  DPR = window.devicePixelRatio;
  W = canvas.width = Math.round(display.w * DPR);
  H = canvas.height = Math.round(display.h * DPR);
  T = W / COLS;
  ROWS = Math.round(H / T);
  TH = H / ROWS;
}

export const layer = (w = W, h = H) => new OffscreenCanvas(Math.ceil(w), Math.ceil(h));
// A still picture, the display's size unless given one, drawn once and frozen into a bitmap: drawing part of a bitmap is
// a cheap copy, where drawing part of a live canvas can cost a snapshot of the whole canvas every time.
export const picture = (draw: (g: OffscreenCanvasRenderingContext2D) => void, w = W, h = H): Picture => {
  const c = layer(w, h);
  draw(c.getContext("2d")!);
  return c.transferToImageBitmap();
};
/** Draws every picture once into one pixel: drawing a picture at all readies it, so no frame waits on uploading one. */
export function warm(pictures: CanvasImageSource[]) {
  for (const img of pictures) screen.drawImage(img, 0, 0, 1, 1);
  screen.clearRect(0, 0, 1, 1);
  screen.getImageData(0, 0, 1, 1);
}
export const load = (src: string) =>
  new Promise<HTMLImageElement>((ok, fail) => {
    const i = new Image();
    i.onload = () => ok(i);
    i.onerror = fail;
    i.src = src;
  });
// Global points to this display's device pixels.
export const lx = (x: number) => (x - display.x) * DPR;
export const ly = (y: number) => (y - display.y) * DPR;
// Copies the part of img at (sx, sy, sw, sh) to (dx, dy, dw, dh), trimming both to the part inside img: WebKit draws
// nothing at all when the source runs even a fraction of a pixel past img's edge.
export function cut(
  g: Context,
  img: Picture,
  sx: number,
  sy: number,
  sw: number,
  sh: number,
  dx: number,
  dy: number,
  dw: number,
  dh: number,
) {
  const x0 = Math.max(0, sx);
  const y0 = Math.max(0, sy);
  const x1 = Math.min(img.width, sx + sw);
  const y1 = Math.min(img.height, sy + sh);
  if (x1 <= x0 || y1 <= y0) return;
  const kx = dw / sw;
  const ky = dh / sh;
  g.drawImage(img, x0, y0, x1 - x0, y1 - y0, dx + (x0 - sx) * kx, dy + (y0 - sy) * ky, (x1 - x0) * kx, (y1 - y0) * ky);
}

// Rings of squares on a grid: every grid square within `band` of radius R, sized by how close it is.
export function ring(g: Context, cx: number, cy: number, R: number, band: number, pitch: number, color: string) {
  g.fillStyle = color;
  g.beginPath();
  const outer = R + band;
  const inner = Math.max(0, R - band);
  const gx = cx - Math.ceil(outer / pitch) * pitch;
  const gy = cy - Math.ceil(outer / pitch) * pitch;
  const onGrid = (v: number, origin: number) => origin + Math.ceil((v - origin) / pitch) * pitch;
  for (let y = onGrid(Math.max(gy, -pitch), gy); y <= Math.min(cy + outer, H + pitch); y += pitch) {
    const dy = y - cy;
    const xo = Math.sqrt(Math.max(0, outer * outer - dy * dy));
    const xi = Math.abs(dy) < inner ? Math.sqrt(inner * inner - dy * dy) : 0;
    for (const [a, b] of [
      [cx - xo, cx - xi],
      [cx + xi, cx + xo],
    ] as const) {
      for (let x = onGrid(Math.max(a, -pitch), gx); x <= Math.min(b, W + pitch); x += pitch) {
        const d = Math.abs(Math.hypot(x - cx, dy) - R);
        if (d > band) continue;
        const s = pitch * 0.86 * (1 - d / band) ** 0.6;
        g.rect(x - s / 2, y - s / 2, s, s);
      }
    }
  }
  g.fill();
}

// A solid disc of grid squares, its edge stippled into smaller squares like the logo's ink. A row's run of whole
// squares goes down as one rectangle.
export function disc(g: Context, cx: number, cy: number, r: number, pitch: number, color: string) {
  g.fillStyle = color;
  g.beginPath();
  for (let y = Math.max(0, Math.floor((cy - r) / pitch) * pitch); y < Math.min(H, cy + r); y += pitch) {
    let whole: number | null = null;
    let x = Math.max(0, Math.floor((cx - r) / pitch) * pitch);
    for (; x < Math.min(W, cx + r); x += pitch) {
      const inside = r - Math.hypot(x + pitch / 2 - cx, y + pitch / 2 - cy);
      if (inside > pitch * 2.1) {
        whole ??= x;
        continue;
      }
      if (whole !== null) g.rect(whole - 0.25, y - 0.25, x - whole + 0.5, pitch + 0.5);
      whole = null;
      if (inside <= 0) continue;
      const s = pitch * Math.min(1, 0.3 + inside / (pitch * 3));
      g.rect(x + (pitch - s) / 2, y + (pitch - s) / 2, s, s);
    }
    if (whole !== null) g.rect(whole - 0.25, y - 0.25, x - whole + 0.5, pitch + 0.5);
  }
  g.fill();
}

/** Jolts `g` up to `amount` device pixels each way, to a new spot every 60th of a second. */
export function jitter(g: Context, t: number, amount: number) {
  const q = Math.floor(t * 60);
  g.translate(amount * (hash(q) - 0.5), amount * (hash(q + 9) - 0.5));
}

/** Shakes `g` on the claps, by up to `amount` of a tile. */
export const shake = (g: Context, t: number, amount: number) => jitter(g, t, T * pulse("clap", t, 0.06) * amount);

/** Paths grouped by opacity rounded to `levels` steps, so a frame of many faint shapes fills or strokes a few paths. */
export function byOpacity(levels: number) {
  const paths = new Map<number, Path2D>();
  const at = (alpha: number) => {
    const level = Math.round(alpha * levels) / levels;
    if (level <= 0) return null;
    const path = paths.get(level) ?? new Path2D();
    paths.set(level, path);
    return path;
  };
  return { at, paths };
}
