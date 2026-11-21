// octopus's own window as the show sees it: its frame and a picture of it, taken just before the octopus swallows the
// screen, and where each place the tour works sits in it, as the window tells.

import { TITLE_H, type ShowApp, type ShowRect, type ShowTarget } from "../generated";
import { DPR, load, lx, ly, warm, type Box, type Context, type Picture } from "./display";

/** The window's corner radius, in its points. */
const RADIUS = 10;

let taken: { frame: ShowRect; img: Picture } | null = null;
/** Whether octopus has handed its window over: the page draws nothing from the dive on until it has, and octopus ends
 * the show if it cannot. */
export const hasApp = () => taken !== null;
/** The window's frame and its picture, decoded, which every scene from the dive on draws from. */
export const app = () => taken!;
/** Where each place the tour works sits in the window, in its points, as the window last told. */
export const spots = new Map<ShowTarget, ShowRect>();

// The picture decodes off the frame loop and is drawn into one pixel as it arrives, so the dive's first frame finds it
// ready.
export async function takeApp({ frame, picture }: ShowApp) {
  const img = await createImageBitmap(await load(picture));
  warm([img]);
  taken = { frame, img };
}

/** A rect of the window, in its points, in this display's device pixels. */
export function inWindow(frame: ShowRect, r: ShowRect): Box {
  return { x: lx(frame.x + r.x), y: ly(frame.y + r.y), w: r.width * DPR, h: r.height * DPR };
}

/** The whole window in this display's device pixels. */
export const windowBox = (frame: ShowRect) => inWindow(frame, { x: 0, y: 0, width: frame.width, height: frame.height });

/** Adds the window's outline at `box`, `zoom` times its size, to `path`. */
export const outline = (path: Path2D | Context, box: Box, zoom = 1) =>
  path.roundRect(box.x, box.y, box.w, box.h, RADIUS * DPR * zoom);

/** The centre of the window in this display's device pixels. */
export function appCentre(): [number, number] {
  const f = app().frame;
  return [lx(f.x + f.width / 2), ly(f.y + f.height / 2)];
}

/** The window's picture at `box`, `zoom` times its size, with the traffic lights the picture leaves to the system. */
export function windowPicture(g: Context, img: Picture, box: Box, zoom: number) {
  const k = DPR * zoom;
  g.save();
  g.beginPath();
  outline(g, box, zoom);
  g.clip();
  g.drawImage(img, box.x, box.y, box.w, box.h);
  ["#ff5f57", "#febc2e", "#28c840"].forEach((fill, i) => {
    g.fillStyle = fill;
    g.beginPath();
    g.arc(box.x + (21 + i * 20) * k, box.y + ((TITLE_H - 1) / 2) * k, 6 * k, 0, Math.PI * 2);
    g.fill();
  });
  g.restore();
}
