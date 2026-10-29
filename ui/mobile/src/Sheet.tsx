import {
  createContext,
  createEffect,
  For,
  on,
  onCleanup,
  onMount,
  Show,
  useContext,
  type Accessor,
  type JSX,
} from "solid-js";
import { scale, ui } from "@octopus/shared/state.ts";
import { back, type Toward } from "./nav";
import {
  afterFrame,
  curve,
  draw,
  FADE,
  FADE_MS,
  glide,
  LEAVE,
  PROJECT_S,
  PUSH,
  reduced,
  SHEET,
  SNAPPY,
  type Gliding,
  type Look,
} from "./motion";
import { Circle, Slot } from "./parts";
import { driveSheetBack, releaseSheetBack } from "./stack";
import { owns, type Gesture } from "./touch";
import { releaseFocus } from "./viewport";

/**
 * How a sheet came up and whether it is going: one rises over the screen, and one shown in place of another, toward
 * where `inPlace` says, takes the other's height and grows or shrinks to its own. A leaving sheet calls `gone` once it
 * is off the screen.
 */
export type Presence = { inPlace: Toward | null; leaving: Accessor<boolean>; gone: () => void };
export const SheetPresence = createContext<Presence>();

/** A released sheet closes once its throw would carry it past half its height, or when thrown faster than this. */
const THROW_PX = 900;
/** How much of its height a predictive back gesture at full progress lowers a sheet. */
const PREDICTIVE_REACH = 0.3;

/** The height of the sheet shown last, which the next one shown in its place starts from, and its panel. */
let shownHeight = 0;
let shownPanel: HTMLElement | null = null;
/** How far the page a sheet goes deeper from slides the other way, as the screen under a push does. */
const PARALLAX = "-30%";

/**
 * Rises from the bottom edge over a scrim and the bottom inset. A drag down from its top carries it and letting go
 * closes or settles it by where the throw would land; a tap on the scrim or the OS's back closes it. Its height follows
 * its contents.
 */
export function Sheet(props: { children: JSX.Element; full?: boolean; style?: JSX.CSSProperties }) {
  const presence = useContext(SheetPresence)!;
  let panel!: HTMLDivElement;
  let scrim!: HTMLDivElement;
  let grab!: HTMLDivElement;
  let offset = 0;
  let releaseVelocity = 0;
  let running: Gliding | null = null;
  const height = () => panel.offsetHeight;

  /** The sheet lowered `y` points below its rest, the scrim fading with it; under reduced motion a fade stands in. */
  const look = (y: number, fade = reduced()): Look => {
    const shown = String(Math.min(Math.max(1 - y / height(), 0), 1));
    return [
      [panel, fade ? { translate: "none", opacity: shown } : { translate: `0 ${y}px`, opacity: "1" }],
      [scrim, { opacity: shown }],
    ];
  };
  const pose = (y: number, fade = false) => {
    offset = y;
    draw(look(y, fade));
  };
  let rising = () => {};
  const stop = () => {
    rising();
    if (running) offset = running.stop();
    running = null;
  };
  const settle = (to: number, spring = SHEET, velocity = 0) => {
    stop();
    const run = glide(offset, to, (y) => look(y), reduced() ? FADE : spring, velocity);
    running = run;
    return run.done.then(() => {
      if (running === run) [offset, running] = [to, null];
    });
  };

  /** Contents that change height carry the sheet's top with them. */
  let morph: Animation | null = null;
  const follow = () => {
    const from = morph?.playState === "running" ? panel.offsetHeight : shownHeight;
    morph?.cancel();
    const to = panel.offsetHeight;
    shownHeight = to;
    if (Math.abs(to - from) < 1 || presence.leaving() || reduced()) return;
    morph = panel.animate(
      [
        { height: `${from}px`, top: "auto" },
        { height: `${to}px`, top: "auto" },
      ],
      curve(SNAPPY),
    );
  };

  /**
   * A page shown in place of another slides in from the side it goes toward, deeper from the right and back from
   * the left, as the other slides away and fades; under reduced motion it fades in.
   */
  const turn = (toward: Toward, gone: HTMLElement | null) => {
    if (toward === "here" || gone === null) return;
    const parts = [...panel.children].filter((part) => part !== grab);
    if (reduced()) {
      for (const part of parts) part.animate([{ opacity: 0 }, { opacity: 1 }], FADE_MS);
      return;
    }
    const page = document.createElement("div");
    page.className = "page-gone";
    page.style.top = `${grab.offsetTop + grab.offsetHeight}px`;
    page.append(...[...gone.children].filter((part) => !part.matches(".grab, .page-gone")));
    panel.append(page);
    const [enters, leaves] = toward === "deeper" ? ["100%", PARALLAX] : [PARALLAX, "100%"];
    for (const part of parts)
      part.animate(
        [{ translate: `${enters} 0`, opacity: toward === "back" ? 0 : 1 }, { translate: "0 0" }],
        curve(PUSH),
      );
    page
      .animate(
        [{ translate: "0 0" }, { opacity: 0, offset: 0.6 }, { translate: `${leaves} 0`, opacity: 0 }],
        curve(PUSH),
      )
      .finished.then(
        () => page.remove(),
        () => page.remove(),
      );
  };

  onMount(() => {
    const contents = new MutationObserver(follow);
    contents.observe(panel, { childList: true, subtree: true });
    onCleanup(() => contents.disconnect());
    const gone = shownPanel;
    shownPanel = panel;
    if (presence.inPlace) {
      follow();
      return turn(presence.inPlace, gone);
    }
    releaseFocus(document.body);
    shownHeight = height();
    pose(height(), reduced());
    rising = afterFrame(() => void settle(0));
  });

  createEffect(
    on(presence.leaving, (leaving) => {
      if (!leaving) return;
      releaseFocus(panel);
      releaseSheetBack(drive);
      void settle(height(), LEAVE, releaseVelocity).then(presence.gone);
    }),
  );

  /** A drag down from the contents' top carries the sheet instead of scrolling them, as does one catching it moving. */
  const zoom = () => scale(ui.prefs);
  const carry: Gesture = ({ axis, way, at }) => {
    if (axis !== "y" || (offset === 0 && (way < 0 || panel.scrollTop > 0))) return null;
    stop();
    const from = offset;
    return {
      move: (to) => pose(Math.max(0, from + (to.y - at.y) / zoom())),
      end: (thrown) => {
        const velocity = (thrown?.y ?? 0) / zoom();
        if (thrown && offset > 0 && (offset + velocity * PROJECT_S > height() / 2 || velocity > THROW_PX)) {
          releaseVelocity = velocity;
          back();
        } else void settle(0, SHEET, velocity);
      },
    };
  };

  const drive = {
    start: stop,
    move: (progress: number) => pose(progress * PREDICTIVE_REACH * height()),
    cancel: () => void settle(0),
  };
  driveSheetBack(drive);
  onCleanup(() => releaseSheetBack(drive));

  return (
    <div class="layer" classList={{ leaving: presence.leaving() }}>
      <div ref={scrim} class="scrim" style={{ opacity: presence.inPlace ? 1 : 0 }} onClick={back} />
      <div
        ref={(element) => {
          panel = element;
          owns(element, carry);
        }}
        class="sheet"
        classList={{ full: props.full }}
        style={props.style}
      >
        <div ref={grab} class="grab" />
        {props.children}
      </div>
    </div>
  );
}

/** A sheet's header: back when it shows a page in place, a lead glyph, its subject over its facts in mono, an action. */
export function SheetHead(props: {
  title: JSX.Element;
  facts?: JSX.Element[];
  back?: () => void;
  lead?: JSX.Element;
  end?: JSX.Element;
}) {
  return (
    <div class="shead" classList={{ lead: props.lead !== undefined && (props.facts?.length ?? 0) > 0 }}>
      <Show when={props.lead}>
        <Slot>{props.lead}</Slot>
      </Show>
      <Show when={props.back}>{(back) => <Circle icon="back" label="Back" tap={back()} />}</Show>
      <div class="ttl">
        <b class="name">{props.title}</b>
        <For each={props.facts}>{(fact) => <span class="id">{fact}</span>}</For>
      </div>
      {props.end}
    </div>
  );
}
