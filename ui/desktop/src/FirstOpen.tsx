import { createComputed, createEffect, createRoot, createSignal, on, onCleanup, onMount, Show } from "solid-js";
import { emit, listen } from "@tauri-apps/api/event";
import { reduced } from "@octopus/shared/motion.ts";
import {
  model,
  report,
  serverView,
  setNewTaskSection,
  showModal,
  showSidebar,
  ui,
  uiAct,
} from "@octopus/shared/state.ts";
import { desktop, desktopEvents } from "./backend";
import { nextBeat, type Beat } from "./beats";
import { titleHeight } from "./layer";
import type { SetupView } from "./generated";
import fibre from "../../design/desktop/fibre.webp?url";
import { inOut } from "./octopus/ease";
import { paintMark, Stage, type Motion, type Point, type Rect } from "./octopus/stage";

/** The install this launch runs: undefined until read, null on a launch that installs nothing. */
const [setup, setSetup] = createSignal<SetupView | null | undefined>(undefined);

/** Follows the install of this machine's server, once its state is read; an event heard first stands. */
export async function followSetup() {
  let heard = false;
  await desktopEvents.setup((view) => {
    heard = true;
    setSetup(view);
  });
  const view = await desktop.setup();
  if (!heard) setSetup(view);
}

/** The server has answered once the client's model, which lists the servers yet to answer, no longer names it. */
const answered = (server: string) =>
  model.version >= 0 && serverView(server) !== undefined && !model.unread.includes(server);

/** Every layer follows the same events, so each reads the same beat. */
export const beat = createRoot(() => {
  const [now, setNow] = createSignal<Beat>("wake");
  createComputed(() => {
    const install = setup();
    if (install === undefined) return;
    const facts = {
      installs: install !== null,
      answered: install !== null && answered(install.server),
      sidebar: ui.sidebar,
      workspaces: model.lists.workspaces.length,
      tasks: Object.keys(model.tasks).length,
    };
    setNow((was) => nextBeat(was, facts));
  });
  return now;
});

/** Whether the octopus has landed in the title bar's mark, and whether the first open still draws. */
const [landed, setLanded] = createSignal(false);
const [out, setOut] = createSignal(true);
/** The title bar's mark shows once the octopus has landed in it, or on a launch with no first open. */
const markShown = () => beat() === "off" || landed();

/** The sidebar stays pinned from the reach until the first task is created. */
export const holdsSidebar = () => ["reach", "offer", "hand_off"].includes(beat());
/** While it offers, the sidebar holds "+ workspace" alone, in its place. */
export const offering = () => ["wake", "reach", "offer"].includes(beat());

/**
 * The float layer tells the base layer where "+ workspace" rests, at the sidebar's right edge, when asked and whenever
 * it may have moved, and when the sidebar starts sliding out, so the tentacle draws it as it goes.
 */
const OFFER = "first-open:offer";
const ASK = "first-open:ask";
const SLIDING = "first-open:sliding";

/** Where "+ workspace" rests, in the window's CSS pixels, from the sidebar's untransformed box. */
function offerPlace(): Point | null {
  const button = document.querySelector<HTMLElement>(".sidebar .addws");
  const aside = button?.closest<HTMLElement>(".sidebar");
  if (!button || !aside) return null;
  const slid = new DOMMatrix(getComputedStyle(aside).transform).m41;
  const [box, rect] = [aside.getBoundingClientRect(), button.getBoundingClientRect()];
  return { x: box.right - slid, y: rect.top + rect.height / 2 };
}

/**
 * The float layer's part: it tells the base layer where "+ workspace" rests, the new-task form opens on the first
 * workspace, and the sidebar lets go after the task.
 */
export function FirstOpenFloat() {
  const tell = () => requestAnimationFrame(() => offerPlace() && void emit(OFFER, offerPlace()));
  const sliding = (event: TransitionEvent) => {
    const target = event.target as HTMLElement;
    const out = target.matches(".sidebar.offering:not(.hidden)") && event.propertyName === "transform";
    if (out) void emit(SLIDING);
  };
  onMount(() => {
    const asked = listen(ASK, tell);
    document.addEventListener("transitionstart", sliding);
    onCleanup(() => {
      void asked.then((unlisten) => unlisten());
      document.removeEventListener("transitionstart", sliding);
    });
    void document.fonts.ready.then(tell);
  });
  createEffect(on(() => [ui.viewport.width, ui.viewport.height, ui.prefs.zoom, beat()], tell));
  createEffect(
    on(beat, (now, was) => {
      if (was === "offer" && now === "hand_off") {
        setNewTaskSection(model.lists.workspaces[0]?.name ?? null);
        showModal("new_task");
      }
      if (was === "hand_off" && now === "done" && !ui.prefs.pinned) showSidebar(false);
    }),
  );
  return null;
}

const token = (name: string) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();
const css = (name: string) => parseFloat(token(name));
/** A time token in seconds, which WebKitGTK may give in seconds and other engines as written, in milliseconds. */
const seconds = (name: string) => parseFloat(token(name)) / (token(name).endsWith("ms") ? 1000 : 1);

/** The title bar's mark: the octopus on its paper tile, hidden while the first open's octopus is still out. */
export function Mark() {
  let canvas!: HTMLCanvasElement;
  const paint = () => paintMark(canvas);
  onMount(() => {
    paint();
    addEventListener("resize", paint);
    onCleanup(() => removeEventListener("resize", paint));
  });
  return <canvas ref={canvas} class="mark" classList={{ away: !markShown() }} />;
}

/** The base layer's part, which exists only while the first open is out. */
export function FirstOpen() {
  return (
    <Show when={out() && beat() !== "off"}>
      <Scene />
    </Show>
  );
}

/**
 * The paper window, the octopus at its middle with its one line, and the stage its tentacles draw on. A key or a
 * click lands the motion under way where it ends; the window then stands where that motion ends.
 */
function Scene() {
  let card!: HTMLDivElement;
  let paper!: HTMLDivElement;
  let stage!: Stage;
  const [offer, setOffer] = createSignal<Point | null>(null);
  let found!: (at: Point) => void;
  const offered = new Promise<Point>((resolve) => (found = resolve));
  const [title, setTitle] = createSignal("");
  /**
   * The checkbox from the wake until the first workspace is added: whether the app keeps the octopus skill for the
   * user's Claude Code, read once it leaves.
   */
  const [drive, setDrive] = createSignal(true);
  const choose = () => {
    uiAct({ do: "set_skill", skill: drive() });
    if (drive()) desktop.skillInstall().catch(report);
  };
  const [leaving, setLeaving] = createSignal(false);
  const size = () => ({ width: ui.viewport.width || innerWidth, height: ui.viewport.height || innerHeight });
  /** The octopus's span and centre: it and its line sit together at the middle of the window below the title bar. */
  const span = () => Math.min(size().width * 0.24, (size().height - titleHeight() - css("--status-h")) * 0.4);
  const lineGap = () => css("--s4");
  const middle = (): Point => {
    const [top, bottom] = [titleHeight(), size().height - css("--status-h")];
    return { x: size().width / 2, y: (top + bottom) / 2 - (lineGap() + css("--fs-ui") * css("--lh-ui")) / 2 };
  };
  /** Clear of the sidebar's edge, the tentacle's tip rests beside "+ workspace". */
  const gap = () => css("--s3");

  let finish = () => {};
  let playing = false;
  /** Runs one motion, which a key or a click lands where it ends; each step starts once the one before it lands. */
  async function play(steps: (() => Motion)[], landing: () => void) {
    let [stopped, current] = [false, null as Motion | null];
    const land = () => {
      if (stopped) return;
      [stopped, playing, finish] = [true, false, () => {}];
      landing();
    };
    playing = true;
    finish = () => (current?.finish(), land());
    for (const step of steps) {
      if (stopped) return;
      current = step();
      await current.done;
    }
    land();
  }

  const rest = () => {
    const at = offer();
    if (at !== null) stage.release(at, gap()).finish();
  };

  /** Lands once the float layer's sidebar starts sliding out, the tentacle holding its edge until then. */
  let slid = () => {};
  const slide = (): Motion => {
    const done = new Promise<void>((resolve) => (slid = resolve));
    showSidebar(true);
    return { finish: slid, done };
  };

  const reach = async () => {
    const at = await offered;
    const pull = seconds("--t-pull");
    void play(
      [
        () => stage.grow({ x: 0, y: at.y }),
        slide,
        () => stage.pull({ x: 0, y: at.y }, at, pull),
        () => stage.release(at, gap()),
      ],
      () => {
        showSidebar(true);
        rest();
      },
    );
  };

  /** The octopus contracts into the title bar's mark. */
  const contract = () => {
    const mark = document.querySelector<HTMLElement>(".chrome .mark")!;
    const box = mark.getBoundingClientRect();
    if (reduced()) setLanded(true);
    return stage.contract(
      { x: box.x, y: box.y, w: box.width, h: box.height },
      parseFloat(getComputedStyle(mark).borderTopLeftRadius),
    );
  };
  /** The octopus stands in the mark, which shows in its place, and the window lets go of the first open. */
  const land = () => {
    contract().finish();
    setLanded(true);
    setTimeout(() => setOut(false), seconds("--t-hover") * 1000 + 50);
  };

  /** The paper fades away, and the window under it takes the pointer. */
  const leave = () => {
    setLeaving(true);
    const fade = seconds("--t-paper");
    stage.tween(fade, (t) => (paper.style.opacity = `${1 - inOut(t / fade)}`));
  };

  /** The tentacle takes the new task from beside the octopus into the terminal pane, where its agent starts. */
  const carry = () => {
    const pane = document.querySelector(".task-split > :last-child")!.getBoundingClientRect();
    const [width, height] = [card.offsetWidth, card.offsetHeight];
    const [at, scale] = [middle(), span() / 960];
    const from: Rect = { x: at.x + 380 * scale, y: at.y - 360 * scale, w: width, h: height };
    const to: Rect = {
      x: pane.x + (pane.width - width) / 2,
      y: pane.y + css("--tabs-h") + css("--s6"),
      w: width,
      h: height,
    };
    setTitle(Object.values(model.tasks)[0]?.task.title ?? "");
    leave();
    const show = (place: Rect, seen: number) => {
      card.style.transform = `translate(${place.x}px, ${place.y}px) scale(${0.96 + 0.04 * seen})`;
      card.style.opacity = `${seen}`;
    };
    void play([() => stage.carry(from, to, css("--r2"), show), contract], land);
  };

  createEffect(
    on(beat, (now, was) => {
      const waking = was === undefined || was === "wake";
      if (["wake", "reach", "offer"].includes(was ?? "") && (now === "hand_off" || now === "done")) choose();
      if (waking && now === "reach") void reach();
      if (waking && (now === "offer" || now === "hand_off")) showSidebar(true);
      if (waking && now === "offer") rest();
      if (now === "hand_off" && !waking) {
        finish();
        void play([() => stage.recoil("leftBottom")], () => {});
      }
      if (was === "hand_off" && now === "done") {
        finish();
        carry();
      }
      if ((waking || was === "offer") && now === "done") {
        finish();
        leave();
        void play([() => stage.recoil("leftBottom"), contract], land);
      }
    }),
  );
  /** A new window size or a new place for "+ workspace" moves the octopus at rest, and its tentacle with it. */
  createEffect(
    on([size, offer], () => {
      stage.resize(size().width, size().height);
      if (playing || stage.moving()) return;
      stage.place(middle(), span());
      if (beat() === "offer") rest();
    }),
  );
  onMount(() => {
    /** The paper's fibre soaks in once it has decoded, so the paper never changes at once. */
    const image = new Image();
    image.src = fibre;
    const soak = seconds("--t-paper");
    const print = () => stage.tween(soak, (t) => paper.style.setProperty("--printed", `${inOut(t / soak)}`));
    void image.decode().then(print, print);
    /** A key or a click on the checkbox only toggles it. */
    const skip = (event: Event) => (event.target as Element).closest(".octo-check") ?? finish();
    document.addEventListener("pointerdown", skip, { capture: true });
    document.addEventListener("keydown", skip, { capture: true });
    const told = listen<Point>(OFFER, (event) => (setOffer(event.payload), found(event.payload)));
    void told.then(() => emit(ASK));
    const heard = listen(SLIDING, () => slid());
    onCleanup(() => {
      document.removeEventListener("pointerdown", skip, { capture: true });
      document.removeEventListener("keydown", skip, { capture: true });
      void told.then((unlisten) => unlisten());
      void heard.then((unlisten) => unlisten());
    });
  });

  const failure = () => {
    const progress = setup()?.progress;
    return progress?.state === "failed" ? progress.error : null;
  };
  return (
    <div class="first-open" classList={{ leaving: leaving() }}>
      <div ref={paper} class="paper" />
      <div ref={card} class="carried" style={{ opacity: 0 }}>
        <span class="spin" />
        {title()}
      </div>
      <canvas
        ref={(canvas) => {
          stage = new Stage(canvas, reduced());
          onCleanup(() => stage.stop());
        }}
      />
      <div class="octo-line" style={{ top: `${middle().y + span() / 2 + lineGap()}px` }}>
        <div classList={{ shown: beat() === "wake" }}>
          <Show when={failure()} fallback={<span>setting up {setup()?.server}…</span>}>
            {(error) => (
              <div class="octo-failed">
                <span class="cause">{error()}</span>
                <span class="log">{setup()?.log}</span>
                <button class="btn live" onClick={() => desktop.setupRetry().catch(report)}>
                  Retry
                </button>
              </div>
            )}
          </Show>
        </div>
        <div classList={{ shown: beat() === "offer" }}>Add a workspace to start a task.</div>
      </div>
      <label class="check octo-check" classList={{ shown: offering() }}>
        <input type="checkbox" checked={drive()} onChange={(event) => setDrive(event.currentTarget.checked)} />
        <span>
          Let Claude Code drive octopus
          <small>adds the octopus skill to ~/.claude</small>
        </span>
      </label>
    </div>
  );
}
