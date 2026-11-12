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
import { back } from "./nav";
import { curve, FADE, PROJECT_S, reduced, SNAPPY, type Toward } from "@octopus/shared/motion.ts";
import { afterFrame, draw, glide, LEAVE, SHEET, turnPage, type Gliding, type Look } from "./motion";
import { Circle, Slot } from "./parts";
import type { BackDrive } from "./stack";
import { owns, type Gesture } from "./touch";
import { releaseFocus } from "./viewport";

/**
 * A sheet as drawn: its panel and the page in it once mounted, the height it last laid out at, how far its panel is
 * scrolled, and what carries it back under Android's back gesture once mounted.
 */
export type Drawn = {
  panel: HTMLElement | null;
  page: HTMLElement | null;
  height: number;
  scrolled: number;
  back: BackDrive | null;
};

/**
 * How a sheet came up and whether it is going: one rises over the screen or over the sheet `over`, and one shown in
 * place of another, toward where `replacing` says, takes the other's height and grows or shrinks to its own. A sheet
 * keeps how it is drawn in `drawn`, and a leaving one calls `gone` once it is off the screen.
 */
type Presence = {
  replacing: { toward: Toward; drawn: Drawn } | null;
  over: Drawn | null;
  drawn: Drawn;
  leaving: Accessor<boolean>;
  gone: () => void;
};
export const SheetPresence = createContext<Presence>();

/** Lets a scroller inside a sheet hand a drag down from its top to the sheet, as the sheet's own top does. */
export const SheetScroller = createContext<(scroller: HTMLElement) => void>(() => {});

/** A released sheet closes once its throw would carry it past half its height, or when thrown faster than this. */
const THROW_PX = 900;
/** How much of its height a predictive back gesture at full progress lowers a sheet. */
const PREDICTIVE_REACH = 0.3;
/** How far a sheet another rises over shrinks back from its top, as iOS stacks its sheets. */
const RECEDE = 0.08;

/**
 * Rises from the bottom edge over a scrim and the bottom inset, or over the sheet up, which shrinks back behind it as
 * it rises and comes forward as it lowers. A drag down from its top carries it and letting go closes or settles it by
 * where the throw would land; a tap on the scrim or the OS's back closes it. Its height follows its contents, and a
 * full one stands from under the top inset to the bottom edge.
 */
export function Sheet(props: { children: JSX.Element; full?: boolean; style?: JSX.CSSProperties }) {
  const presence = useContext(SheetPresence)!;
  const drawn = presence.drawn;
  let panel!: HTMLDivElement;
  let page!: HTMLDivElement;
  let scrim!: HTMLDivElement;
  let offset = 0;
  let releaseVelocity = 0;
  let running: Gliding | null = null;
  const height = () => panel.offsetHeight;

  /**
   * The sheet lowered `y` points below its rest, the scrim fading with it and the sheet under it coming forward;
   * under reduced motion a fade stands in.
   */
  const look = (y: number, fade = reduced()): Look => {
    const shown = Math.min(Math.max(1 - y / height(), 0), 1);
    const under = presence.over?.panel;
    return [
      [panel, fade ? { translate: "none", opacity: String(shown) } : { translate: `0 ${y}px`, opacity: "1" }],
      [scrim, { opacity: String(shown) }],
      ...(under ? [[under, { scale: fade ? "1" : String(1 - RECEDE * shown) }] as Look[number]] : []),
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

  /** Contents that change height carry the sheet's top with them; a full sheet's height is the screen's. */
  let morph: Animation | null = null;
  const follow = () => {
    const from = morph?.playState === "running" ? panel.offsetHeight : drawn.height;
    morph?.cancel();
    const to = panel.offsetHeight;
    drawn.height = to;
    if (Math.abs(to - from) < 1 || presence.leaving() || reduced()) return;
    morph = panel.animate(
      [
        { height: `${from}px`, top: "auto" },
        { height: `${to}px`, top: "auto" },
      ],
      curve(SNAPPY),
    );
  };

  onMount(() => {
    drawn.panel = panel;
    drawn.page = page;
    drawn.back = drive;
    if (!props.full) {
      const contents = new MutationObserver(follow);
      contents.observe(panel, { childList: true, subtree: true });
      onCleanup(() => contents.disconnect());
    }
    if (presence.replacing) {
      const { toward, drawn: was } = presence.replacing;
      drawn.height = was.height;
      follow();
      if (was.page) turnPage(page, was.page, toward, was.scrolled);
      return;
    }
    // A sheet rising over another lets its own field take the focus straight from the other's, keeping the keyboard up.
    const under = presence.over?.panel;
    if (under) queueMicrotask(() => releaseFocus(under));
    else releaseFocus(document.body);
    drawn.height = height();
    pose(height(), reduced());
    rising = afterFrame(() => void settle(0));
  });

  createEffect(
    on(presence.leaving, (leaving) => {
      if (!leaving) return;
      releaseFocus(panel);
      void settle(height(), LEAVE, releaseVelocity).then(presence.gone);
    }),
  );

  /**
   * A drag down from the top of `contents`, the sheet or a scroller in it, carries the sheet instead of scrolling them,
   * as does one catching it moving.
   */
  const zoom = () => scale(ui.prefs);
  const carry =
    (contents: HTMLElement): Gesture =>
    ({ axis, way, at }) => {
      if (axis !== "y" || (offset === 0 && (way < 0 || contents.scrollTop > 0))) return null;
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

  const drive: BackDrive = {
    start: stop,
    move: (progress: number) => pose(progress * PREDICTIVE_REACH * height()),
    cancel: () => void settle(0),
  };

  return (
    <div class="layer" classList={{ leaving: presence.leaving() }}>
      <div ref={scrim} class="scrim" style={{ opacity: presence.replacing ? 1 : 0 }} onClick={back} />
      <div
        ref={(element) => {
          panel = element;
          owns(element, carry(element));
        }}
        class="sheet"
        classList={{ full: props.full, over: presence.over !== null }}
        style={props.style}
        onScroll={() => (drawn.scrolled = panel.scrollTop)}
      >
        <div class="grab" />
        <div ref={page} class="page">
          <SheetScroller.Provider value={(scroller) => owns(scroller, carry(scroller))}>
            {props.children}
          </SheetScroller.Provider>
        </div>
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
