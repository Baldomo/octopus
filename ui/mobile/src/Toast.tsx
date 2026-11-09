import { createEffect, createSignal, on, Show } from "solid-js";
import { scale, ui } from "@octopus/shared/state.ts";
import { feel } from "./feel";
import { Icon } from "./Icon";
import { play, reduced, SNAPPY, tracker } from "@octopus/shared/motion.ts";
import { glide, type Gliding } from "./motion";
import { Slot } from "./parts";

const SAID_MS = 4000;
/** A toast swiped down further than this, or thrown down, goes. */
const SWIPE_PX = 24;
const THROW_PX = 400;
/** A toast dragged up follows the finger this many times less far. */
const UP_GIVE = 4;

const [said, setSaid] = createSignal<{ text: string; nth: number } | null>(null);

/** Confirms what an action did, briefly, with the OS's success feel. */
export function notify(text: string) {
  const nth = (said()?.nth ?? 0) + 1;
  feel("success");
  setSaid({ text, nth });
  setTimeout(() => said()?.nth === nth && setSaid(null), SAID_MS);
}

export async function copy(text: string, what: string) {
  await navigator.clipboard.writeText(text);
  notify(`Copied ${what}`);
}

/**
 * A failed op, with its warning and the OS's error feel, or a confirmation, 8 above the bottom edge and 8 in from the
 * sides. It rises in, its text changes in place, and it sinks away when it ends or a finger swipes it down.
 */
export function Toast() {
  const [shown, setShown] = createSignal<{ text: string; problem: boolean } | null>(null);
  /** A problem swiped away stays away until another takes its place. */
  const [dismissed, setDismissed] = createSignal<string | null>(null);
  let toast: HTMLDivElement | undefined;
  let leaving: Animation | null = null;
  const wanted = () => {
    const problem = ui.problem === dismissed() ? null : ui.problem;
    const text = problem ?? said()?.text;
    return text === undefined || text === null ? null : { text, problem: problem !== null };
  };
  createEffect(
    on(
      () => ui.problem,
      (problem) => (problem === null ? setDismissed(null) : feel("error")),
      { defer: true },
    ),
  );
  createEffect(
    on(wanted, (next) => {
      if (next !== null) {
        leaving?.cancel();
        leaving = null;
        if (shown() !== null && toast) play(toast.lastElementChild!, [{ opacity: 0 }, { opacity: 1 }], SNAPPY);
        setShown(next);
        return;
      }
      if (!toast || shown() === null) return;
      leaving = play(toast, [{}, { transform: "translateY(calc(100% + 16px))", opacity: 0 }], SNAPPY);
      leaving.finished.then(gone, () => {});
    }),
  );

  const gone = () => {
    setShown(null);
    toast = undefined;
  };
  let finger: { y: number; down: number; track: ReturnType<typeof tracker> } | null = null;
  /** A released toast on its way back or out, which a finger landing on it catches where it stands. */
  let settling: Gliding | null = null;
  const dismiss = () => {
    if (shown()?.problem) setDismissed(shown()!.text);
    else setSaid(null);
    gone();
  };
  return (
    <Show when={shown()}>
      {(now) => (
        <div
          ref={(element) => {
            toast = element;
            // An animation started before the element is in the page holds its first frame.
            queueMicrotask(() =>
              play(element, [{ transform: "translateY(calc(100% + 16px))", opacity: 0 }, {}], SNAPPY),
            );
          }}
          class="toast"
          onTouchStart={(event) => {
            const y = event.touches[0]?.clientY;
            if (y === undefined) return;
            const caught = settling?.stop() ?? 0;
            settling = null;
            const travel = caught > 0 ? caught : caught * UP_GIVE;
            finger = { y: y - travel * scale(ui.prefs), down: caught, track: tracker() };
          }}
          onTouchMove={(event) => {
            const y = event.touches[0]?.clientY;
            if (!finger || y === undefined || !toast) return;
            finger.track.add(0, y);
            const down = (y - finger.y) / scale(ui.prefs);
            finger.down = down > 0 ? down : down / UP_GIVE;
            toast.style.translate = `0 ${finger.down}px`;
          }}
          onTouchEnd={() => {
            if (!finger || !toast) return;
            const element = toast;
            const down = finger.down;
            const velocity = finger.track.velocity().y / scale(ui.prefs);
            finger = null;
            const gone = down > SWIPE_PX || velocity > THROW_PX;
            const to = gone ? element.offsetHeight + 16 : 0;
            if (reduced() && gone) return dismiss();
            const run = glide(down, to, (y) => [[element, { translate: `0 ${y}px` }]], SNAPPY, velocity);
            settling = run;
            void run.done.then(() => {
              if (settling !== run) return;
              settling = null;
              element.style.translate = "";
              if (gone) dismiss();
            });
          }}
        >
          <Show when={now().problem}>
            <Slot>
              <Icon name="warn" />
            </Slot>
          </Show>
          <span class="name">{now().text}</span>
        </div>
      )}
    </Show>
  );
}
