// One display's view of the first open's show. The picture is a pure function of the song's time and the pointer, so
// every display renders its own part in sync from the same clock. The page sees no other app's pixels: it draws over
// the live desktop through its transparent window, and knows the wallpaper and where octopus's window sits.
//
// The infection glitches the desktop, then blacks it out and brings it back as bare wallpaper; something knocks from
// behind it and turns it over to the octopus's world; the octopus charges and swallows the screen; its pupil opens on
// octopus's own window and the show dives into it, and tours it live; the octopus inks every display, and wipes the
// ink off with its arms.

import "../../../design/desktop/tokens.css";
import { listen } from "@tauri-apps/api/event";
import { EVENTS, type ShowApp, type ShowScene, type ShowSpot } from "../generated";
import { hasApp, takeApp, spots } from "./app";
import { invoke } from "./invoke";
import { fit, follow, H, screen, W, warm, type Context } from "./display";
import { finale, prepareFinale } from "./finale";
import { haunt, TURNED, turn } from "./haunt";
import { ink, prepareInk } from "./ink";
import { AT, INFECTION, SHOW } from "./music";
import { prepareStills } from "./stills";
import { dive, STEPS, tour } from "./tour";
import { dark, SQUIRT, world } from "./world";

const SCENES: [number, (g: Context, t: number) => void][] = [
  [0, haunt],
  [AT.turn, turn],
  [TURNED, world],
  [AT.dark, dark],
  [AT.drop, dive],
  [AT.land, tour],
  [AT.ink, ink],
  [AT.back, finale],
];

/** Tells octopus something, logging a refusal: a page of a show that has ended asks after nothing. */
const tell = (command: Parameters<typeof invoke>[0], args?: Record<string, unknown>) =>
  void invoke(command, args).catch((error: unknown) => console.error(`show: ${command}`, error));

/** What the driving page asks of octopus, each once at its moment, in order: the alert sounds, the window just before
 * the octopus swallows the screen, the tour's steps, and the cursor at each change. */
const CUES = [
  ...INFECTION.filter((e) => e.alert).map(({ t }) => ({ at: t, run: () => tell("show_alert") })),
  { at: AT.lunge, run: () => tell("show_app") },
  ...STEPS.map(({ at, step }) => ({ at, run: () => tell("show_drive", { step }) })),
  // The real cursor hides while the show draws the beachball in its place, and from when the octopus swallows it
  // until the ink lands, so the tour's pointer is the only one.
  ...(
    [
      [INFECTION[0]!.t, false],
      [SHOW.musicIn, true],
      [AT.dark - SQUIRT, false],
      [AT.ink, true],
    ] as const
  ).map(([at, shown]) => ({ at, run: () => tell("show_cursor", { shown }) })),
].sort((a, b) => a.at - b.at);

// Nothing is drawn before the song, after it, or from the dive until octopus has handed its window over.
function draw(t: number) {
  screen.setTransform(1, 0, 0, 1, 0, 0);
  if (t < 0 || t >= AT.end || (t >= AT.drop && !hasApp())) return screen.clearRect(0, 0, W, H);
  SCENES.findLast(([from]) => t >= from)![1](screen, t);
}

// Native hands every display the same scene timing: the wall-clock instant the song starts, in ms since the epoch.
// Only the driving page asks octopus for anything.
function play(epochMs: number, cues: typeof CUES) {
  const clock = () => (performance.timeOrigin + performance.now() - epochMs) / 1000;
  let cued = 0;
  const loop = () => {
    const t = clock();
    while (cued < cues.length && cues[cued]!.at <= t) cues[cued++]!.run();
    draw(t);
    requestAnimationFrame(loop);
  };
  loop();
}

// Everything a scene would otherwise build on its first frame is built now, and every picture is drawn once, so no
// frame of the show waits on building or uploading a picture.
async function prepare(scene: ShowScene) {
  fit(scene);
  const stills = await prepareStills(scene);
  const ink = prepareInk();
  warm([...stills, ...ink.pictures, ...prepareFinale(ink.last)]);
}

const skip = () => tell("show_skip");
addEventListener("keydown", skip);
addEventListener("pointerdown", skip);

void (async () => {
  // Under the OS's reduced motion there is no show: the page ends it before it starts.
  if (matchMedia("(prefers-reduced-motion: reduce)").matches) return skip();
  await listen<{ x: number; y: number }>(EVENTS.showPointer, (event) => follow(event.payload));
  await listen<ShowSpot>(EVENTS.showSpot, (event) => spots.set(event.payload.target, event.payload.rect));
  await listen<ShowApp>(EVENTS.showApp, (event) => takeApp(event.payload));
  const scene = await invoke<ShowScene>("show_scene");
  await listen<number>(EVENTS.showPlay, (event) => play(event.payload, scene.drives ? CUES : []));
  await prepare(scene);
  await invoke("show_ready");
})();
