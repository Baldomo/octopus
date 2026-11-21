// The display's still pictures, made once before the show: the bare desktop, the octopus's paper, and the octopus
// asleep on it, which the tiles turn over to. Preparing them also loads the captions' font.

import fibreUrl from "../../../design/desktop/fibre.webp?url";
import interUrl from "@fontsource-variable/inter/files/inter-latin-wght-normal.woff2?url";
import type { ShowScene } from "../generated";
import { octopus, scaleFor } from "./creature";
import { C, H, load, picture, W, type Picture } from "./display";

/** How much of the display's height the octopus fills in its world. */
export const WORLD_SIZE = 0.92;

export let bare: Picture;
export let paper: Picture;
export let reveal: Picture;

/** Makes the stills for the scene's display, and hands them back to be warmed. */
export async function prepareStills(scene: ShowScene): Promise<Picture[]> {
  const inter = new FontFace("Inter", `url(${interUrl})`, { weight: "100 900" });
  const [fibre, wall] = await Promise.all([
    load(fibreUrl),
    scene.wallpaper ? load(scene.wallpaper) : null,
    inter.load().then((face) => document.fonts.add(face)),
  ]);
  bare = picture((g) => {
    // octopus hands the wallpaper over already drawn to cover the display.
    if (wall) return g.drawImage(wall, 0, 0, W, H);
    const fall = g.createLinearGradient(0, 0, 0, H);
    fall.addColorStop(0, "#1d2033");
    fall.addColorStop(1, "#07080d");
    g.fillStyle = fall;
    g.fillRect(0, 0, W, H);
  });
  paper = picture((g) => {
    g.fillStyle = C.paper;
    g.fillRect(0, 0, W, H);
    g.globalCompositeOperation = "multiply";
    g.globalAlpha = 0.5;
    g.fillStyle = g.createPattern(fibre, "repeat")!;
    g.fillRect(0, 0, W, H);
  });
  reveal = picture((g) => {
    g.drawImage(paper, 0, 0);
    octopus(g, { x: W / 2, y: H / 2, scale: scaleFor(WORLD_SIZE), lid: 1 });
  });
  return [bare, paper, reveal];
}
