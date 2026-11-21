// The drop and the tour. The octopus's pupil opens on octopus's own window, and the show dives through it: the window
// starts three times its size, tilted, deep inside the pupil, and settles onto its own frame by the end of bar 9 while
// rings beat out from it. From bar 10 the live window shows through a hole in the navy, and the show makes the user's
// first task in it on the blank tidepool, each act on a beat, a pointer of squares gliding there over the beat
// before, a spotlight round the part at work and a caption beside it naming the step.

import { inOut, lerp, outCubic, smooth } from "../octopus/ease";
import { TOUR, type ShowDrive, type ShowTarget } from "../generated";
import { app, appCentre, inWindow, outline, spots, windowBox, windowPicture } from "./app";
import { C, DPR, H, ring, shake, T, W, type Box, type Context } from "./display";
import { AT, BEAT, hitsUpTo, pulse } from "./music";

/** How long the dive takes to settle onto the window. */
const SETTLE = 1.6;
const DIM = "rgba(10, 11, 24, 0.78)";
/** The moment `beats` beats after the window lands. */
const onBeat = (beats: number) => AT.land + beats * BEAT;

/** Each act, on its beat: where the pointer goes and what the window does there. */
const ACTS: { target: ShowTarget; at: number; act: ShowDrive | null }[] = [
  { target: "edge", at: onBeat(1), act: { do: "sidebar", shown: true } },
  { target: "add_workspace", at: onBeat(2.5), act: { do: "press", target: "add_workspace" } },
  { target: "host", at: onBeat(3.5), act: { do: "press", target: "host" } },
  { target: "folder", at: onBeat(4.5), act: { do: "press", target: "folder" } },
  { target: "choose", at: onBeat(5.5), act: { do: "press", target: "choose" } },
  { target: "new_task", at: onBeat(7), act: { do: "press", target: "new_task" } },
  { target: "title", at: onBeat(8), act: null },
  { target: "create", at: onBeat(11), act: { do: "press", target: "create" } },
  { target: "terminal", at: onBeat(12.5), act: null },
];
/** The parts of the window the spotlight rings, each from its beat. */
const REGIONS: { target: ShowTarget; from: number }[] = [
  { target: "sidebar", from: onBeat(1) },
  { target: "form", from: onBeat(7) },
  { target: "terminal", from: onBeat(11) },
];
const CAPTIONS = [
  { text: "Start with a project.", from: onBeat(1.5) },
  { text: "It becomes a workspace.", from: onBeat(5.5) },
  { text: "Give it a task.", from: onBeat(7.5) },
  { text: "Every task gets its own agent.", from: onBeat(11.5) },
  { text: "And it gets to work.", from: onBeat(15) },
];

/**
 * The steps the show hands octopus's window, each at its moment: the window is handed to the tour while the dive
 * settles, so the first open has stepped aside by bar 10; each act on its beat, the title typed two letters to a 16th;
 * and once the ink drowns the screen the window goes back as it was.
 */
type Step = { at: number; step: ShowDrive };
const step = (at: number, step: ShowDrive): Step => ({ at, step });
export const STEPS: Step[] = [
  step(AT.drop + SETTLE - 0.45, { do: "start" }),
  ...ACTS.flatMap(({ at, act }) => (act ? [step(at, act)] : [])),
  ...Array.from({ length: Math.ceil(TOUR.task.length / 2) }, (_, k) =>
    step(onBeat(8) + (k * BEAT) / 4, { do: "type", target: "title", text: TOUR.task.slice(0, 2 * k + 2) }),
  ),
  step(AT.drown, { do: "end" }),
];

export function dive(g: Context, t: number) {
  const age = t - AT.drop;
  g.fillStyle = C.navy;
  g.fillRect(0, 0, W, H);
  const [cx, cy] = appCentre();
  const { frame, img } = app();
  g.save();
  shake(g, t, 0.5 * (1 - smooth(0, SETTLE, age)));
  beats(g, t, AT.drop, cx, cy);
  const settle = inOut(age / SETTLE);
  const zoom = lerp(3, 1, settle) * (1 + 0.03 * pulse("kick", t, 0.12) * (1 - settle));
  const w = frame.width * DPR * zoom;
  const h = frame.height * DPR * zoom;
  const r = lerp(T * 0.5, Math.hypot(W, H), outCubic(age / 0.7));
  g.save();
  g.beginPath();
  g.arc(cx, cy, r, 0, Math.PI * 2);
  g.clip();
  g.translate(cx, cy);
  g.rotate(lerp(-0.14, 0, settle));
  windowPicture(g, img, { x: -w / 2, y: -h / 2, w, h }, zoom);
  g.restore();
  if (r < Math.hypot(W, H) * 0.9) ring(g, cx, cy, r, T * 0.35, T * 0.25, C.gold);
  g.restore();
  if (age < 0.09) {
    g.fillStyle = age < 0.045 ? C.white : C.paper;
    g.globalAlpha = 1 - age / 0.09;
    g.fillRect(0, 0, W, H);
    g.globalAlpha = 1;
  }
}

export function tour(g: Context, t: number) {
  g.fillStyle = C.navy;
  g.fillRect(0, 0, W, H);
  const [cx, cy] = appCentre();
  beats(g, t, AT.land, cx, cy);
  const win = windowBox(app().frame);
  g.globalCompositeOperation = "destination-out";
  g.beginPath();
  outline(g, win);
  g.fill();
  g.globalCompositeOperation = "source-over";
  const spot = lit(t);
  if (spot) spotlight(g, t, win, spot, smooth(onBeat(1), onBeat(1.4), t) * (1 - smooth(onBeat(18.5), onBeat(19.5), t)));
  pointer(g, t, win);
  const i = CAPTIONS.findLastIndex((c) => c.from <= t);
  const said = CAPTIONS[i];
  if (!said || !spot) return;
  const size = 40 * (win.h / 900) * DPR;
  const max = win.w * 0.42;
  const lasts = (CAPTIONS[i + 1]?.from ?? AT.ink) - said.from;
  const gap = win.w * 0.04;
  const right = spot.x + spot.w + gap + max < win.x + win.w;
  const x = right ? spot.x + spot.w + gap : spot.x - gap;
  caption(
    g,
    said.text.split(" "),
    t - said.from,
    lasts,
    x,
    spot.y + spot.h * 0.45,
    size,
    max,
    right ? "left" : "right",
  );
}

/** Where `target` sits in this display's device pixels, once the window has told. */
function boxOf(target: ShowTarget): Box | null {
  const rect = spots.get(target);
  return rect ? inWindow(app().frame, rect) : null;
}

// The part of the window the spotlight rings at t, gliding from the part before over the beat from its own.
function lit(t: number): Box | null {
  const i = REGIONS.findLastIndex((region) => region.from <= t);
  const was = i > 0 ? boxOf(REGIONS[i - 1]!.target) : null;
  const now = (i >= 0 ? boxOf(REGIONS[i]!.target) : null) ?? was;
  if (!now) return null;
  if (!was) return now;
  const e = inOut((t - REGIONS[i]!.from) / BEAT);
  return { x: lerp(was.x, now.x, e), y: lerp(was.y, now.y, e), w: lerp(was.w, now.w, e), h: lerp(was.h, now.h, e) };
}

// Dims the window but for `spot`, which a frame of the octopus's squares rings, a wave running round it on each kick;
// `on` fades it in and out.
function spotlight(g: Context, t: number, win: Box, spot: Box, on: number) {
  if (on <= 0) return;
  const pad = 8 * DPR;
  g.save();
  g.globalAlpha = on;
  g.fillStyle = DIM;
  g.beginPath();
  outline(g, win);
  g.rect(spot.x - pad, spot.y - pad, spot.w + 2 * pad, spot.h + 2 * pad);
  g.fill("evenodd");
  squareFrame(g, t, spot, pad);
  g.restore();
}

// Touching saffron squares round `b`, `out` beyond its edges; on each kick a swell runs round the frame from its top
// left corner, growing each square it passes.
function squareFrame(g: Context, t: number, b: Box, out: number) {
  const side = 5 * DPR;
  const [x0, y0, x1, y1] = [
    b.x - out - side / 2,
    b.y - out - side / 2,
    b.x + b.w + out + side / 2,
    b.y + b.h + out + side / 2,
  ];
  const edge: [number, number][] = [];
  for (let x = x0; x < x1; x += side) edge.push([x, y0]);
  for (let y = y0; y < y1; y += side) edge.push([x1, y]);
  for (let x = x1; x > x0; x -= side) edge.push([x, y1]);
  for (let y = y1; y > y0; y -= side) edge.push([x0, y]);
  const kick = hitsUpTo("kick", AT.land, t).at(-1);
  const front = kick ? ((t - kick.t) / BEAT) * edge.length : -Infinity;
  g.fillStyle = C.paper;
  edge.forEach(([x, y], k) => {
    const swell = Math.max(0, 1 - Math.abs(k - front) / 10);
    const s = side * (1 + 0.8 * swell);
    g.fillRect(x - s / 2, y - s / 2, s, s);
  });
}

// The pointer, a classic arrow drawn in squares, saffron rimmed in navy: it comes in from under the window on bar 10's
// downbeat, glides to each act's place over at most a beat, arriving a 16th before the act, and on each act dips and
// throws a ring of squares; it fades out on bar 14.
const ARROW = [
  "X",
  "XX",
  "X.X",
  "X..X",
  "X...X",
  "X....X",
  "X.....X",
  "X......X",
  "X.......X",
  "X........X",
  "X.....XXXXX",
  "X..X..X",
  "X.X X..X",
  "XX  X..X",
  "X    X..X",
  "     X..X",
  "      XX",
];
function pointer(g: Context, t: number, win: Box) {
  const fade = 1 - smooth(onBeat(17), onBeat(18), t);
  if (fade <= 0) return;
  let where = { x: win.x + win.w * 0.7, y: win.y + win.h + H * 0.1 };
  let from = AT.land;
  for (const act of ACTS) {
    const box = boxOf(act.target);
    if (!box) continue;
    const to = { x: box.x + box.w / 2, y: box.y + box.h / 2 };
    const arrive = act.at - BEAT / 4;
    const start = Math.max(from, arrive - BEAT);
    if (t < arrive) {
      const e = inOut((t - start) / (arrive - start));
      where = { x: lerp(where.x, to.x, e), y: lerp(where.y, to.y, e) };
      break;
    }
    where = to;
    from = act.at + BEAT / 8;
  }
  const acted = ACTS.find((a) => a.act && t >= a.at && t < a.at + BEAT);
  const age = acted ? t - acted.at : Infinity;
  if (age < BEAT) ring(g, where.x, where.y, H * 0.012 + age * H * 0.12, T * 0.12, T * 0.08, C.paper);
  const q = H / 260;
  const dip = age < BEAT / 4 ? 0.85 : 1;
  g.save();
  g.globalAlpha = fade;
  g.translate(where.x, where.y);
  g.scale(dip, dip);
  ARROW.forEach((row, y) =>
    [...row].forEach((cell, x) => {
      if (cell === " ") return;
      g.fillStyle = cell === "X" ? C.navy : C.paper;
      g.fillRect(x * q, y * q, q + 0.5, q + 0.5);
    }),
  );
  g.restore();
}

// A caption in Inter 800, white with its last word in saffron, wrapped at `max` wide and set from (x, y) to the side
// `align` names; each word rises into place on its own 16th, and the caption lifts away over its last 8th.
function caption(
  g: Context,
  words: string[],
  age: number,
  lasts: number,
  x: number,
  y: number,
  size: number,
  max: number,
  align: "left" | "right",
) {
  g.save();
  g.font = `800 ${size}px Inter, -apple-system, sans-serif`;
  g.textBaseline = "middle";
  const space = g.measureText(" ").width;
  const lines: { word: string; w: number; i: number }[][] = [[]];
  let width = 0;
  words.forEach((word, i) => {
    const w = g.measureText(word).width;
    if (width + w > max && lines.at(-1)!.length) (lines.push([]), (width = 0));
    lines.at(-1)!.push({ word, w, i });
    width += w + space;
  });
  const leave = inOut((age - (lasts - BEAT / 2)) / (BEAT / 2));
  const lead = size * 1.15;
  g.shadowColor = "rgba(0, 0, 0, 0.45)";
  g.shadowBlur = size * 0.4;
  lines.forEach((line, l) => {
    const total = line.reduce((sum, { w }) => sum + w, 0) + space * (line.length - 1);
    let from = align === "left" ? x : x - total;
    const row = y + (l - (lines.length - 1) / 2) * lead - leave * size * 0.6;
    for (const { word, w, i } of line) {
      const u = outCubic((age - (i * BEAT) / 4) / 0.18);
      if (u > 0) {
        g.globalAlpha = u * (1 - leave);
        g.fillStyle = i === words.length - 1 ? C.paper : C.white;
        g.fillText(word, from, row + (1 - u) * size * 0.5);
      }
      from += w + space;
    }
  });
  g.restore();
}

// Rings of squares beating out from (cx, cy) on every kick since `from`.
function beats(g: Context, t: number, from: number, cx: number, cy: number) {
  for (const h of hitsUpTo("kick", from, t)) {
    const age = t - h.t;
    if (age < 1) ring(g, cx, cy, age * H * 1.6, T * (0.7 - 0.6 * age), T * 0.3, h.i % 2 ? C.paper : C.lav);
  }
}
