/**
 * The film's print look on the ink: each ink carries the logo's grain, `ink * (1 + 2 * GRAIN * (g - 0.5))`, and the grain
 * re-seeds a few times a second like printed animation. White stays bare, as the film leaves it.
 */
import grainUrl from "./grain.webp?url";
import { WHITE } from "./body";
import { hash } from "./ease";

const GRAIN = 0.35;
/** Seconds between re-seeds: every seventh frame at 60 fps, free of any rhythm. */
const STEP = 7 / 60;

function canvas(width: number, height: number) {
  const element = document.createElement("canvas");
  [element.width, element.height] = [width, height];
  return element.getContext("2d", { willReadFrequently: true })!;
}

let texture: Promise<ImageData> | undefined;
/** The grain, loaded the first time a stage asks for it. */
async function grainTexture() {
  const image = new Image();
  image.src = grainUrl;
  await image.decode();
  const context = canvas(image.width, image.height);
  context.drawImage(image, 0, 0);
  return context.getImageData(0, 0, image.width, image.height);
}

/** One fill per ink: its grained pattern, or its bare colour for white. */
export type Fills = (CanvasPattern | string)[];

/** The fills for `colors` (one CSS colour per ink) on `context`, once the grain has loaded. */
export async function fills(context: CanvasRenderingContext2D, colors: string[]): Promise<Fills> {
  const grain = await (texture ??= grainTexture());
  const probe = canvas(1, 1);
  return colors.map((color, ink) => {
    if (ink === WHITE) return color;
    probe.fillStyle = color;
    probe.fillRect(0, 0, 1, 1);
    const [r, g, b] = probe.getImageData(0, 0, 1, 1).data;
    const tile = new ImageData(grain.width, grain.height);
    for (let i = 0; i < tile.data.length; i += 4) {
      const k = 1 + 2 * GRAIN * (grain.data[i]! / 255 - 0.5);
      [tile.data[i], tile.data[i + 1], tile.data[i + 2], tile.data[i + 3]] = [r! * k, g! * k, b! * k, 255];
    }
    const paint = canvas(grain.width, grain.height);
    paint.putImageData(tile, 0, 0);
    return context.createPattern(paint.canvas, "repeat")!;
  });
}

/** Moves the grain under every fill to the seed of time t. */
export function reseed(all: Fills, t: number) {
  const seed = Math.floor(t / STEP);
  const shift = new DOMMatrix().translate(Math.floor(hash(seed) * 512), Math.floor(hash(seed + 0.5) * 512));
  for (const fill of all) if (typeof fill !== "string") fill.setTransform(shift);
}
