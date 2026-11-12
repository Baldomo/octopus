import { report } from "@octopus/shared/state.ts";
import { curve, reduced } from "@octopus/shared/motion.ts";
import { KEYBOARD } from "./motion";
import { phone } from "./phone";

/** A viewport shorter than the tallest one seen in this orientation by more than this has the keyboard over it. */
const KEYBOARD_PX = 80;

/** Hands the OS's insets to CSS as `--os-inset-*` where the webview leaves them out of `env(safe-area-inset-*)`. */
const readInsets = () =>
  phone.insets().then((insets) => {
    if (insets === null) return;
    for (const [side, px] of Object.entries(insets)) {
      document.documentElement.style.setProperty(`--os-inset-${side}`, `${px}px`);
    }
  }, report);

/**
 * What sits at the foot of the screen: a task's foot, the unreachable row and the dock, and a sheet short enough to
 * stand on it. A full sheet hangs from under the top inset, so its top stays and its contents end above the keyboard.
 */
const FEET = ".app.shown .foot, .sheet:not(.full)";

/** Where an element's bottom sits in the layout, in its own points, wherever a move in flight draws it. */
function laidOutBottom(element: HTMLElement) {
  let bottom = element.offsetHeight;
  for (let at: HTMLElement | null = element; at !== null; at = at.offsetParent as HTMLElement | null) {
    bottom += at.offsetTop;
  }
  return bottom;
}

/**
 * Lays out at once and lets the foot of the screen glide from where it stood to where the keyboard leaves it, on top of
 * any move it is already making. A glide that knows when the keyboard set off, on the page's clock, keeps pace with it
 * however late the page draws its first frame.
 */
function glideFeet(relayout: () => void, setOff: number | null) {
  const feet = reduced() ? [] : [...document.querySelectorAll<HTMLElement>(FEET)];
  const before = feet.map(laidOutBottom);
  relayout();
  feet.forEach((foot, nth) => {
    const moved = before[nth]! - laidOutBottom(foot);
    if (Math.abs(moved) <= 1) return;
    const glide = foot.animate([{ transform: `translateY(${moved}px)` }, { transform: "none" }], {
      ...curve(KEYBOARD),
      composite: "add",
    });
    if (setOff !== null) glide.startTime = setOff;
  });
}

/** The keyboard types into the field that has the focus, so a screen or sheet that leaves or is covered lets go of it. */
export function releaseFocus(within: Element | undefined) {
  const focused = document.activeElement;
  if (focused instanceof HTMLElement && focused !== document.body && within?.contains(focused)) focused.blur();
}

/**
 * Sizes the app to the part of the screen the on-screen keyboard leaves, so whatever sits at the app's foot sits
 * directly above the keyboard, and marks the root `data-keyboard` while the keyboard is up. iOS tells the page as
 * `octopus-keyboard` where the keyboard is going as it starts to move, so the foot moves with it; the visual viewport
 * reports it only once it stops. The webview scrolls its page to reveal a focused field, which would carry the app
 * under the status bar, so the page is held at the top.
 */
export function followViewport() {
  const viewport = window.visualViewport!;
  const root = document.documentElement;
  let orientation = screen.orientation.type;
  let tallest = 0;
  const follow = (height: number, setOff: number | null) => {
    if (screen.orientation.type !== orientation) [orientation, tallest] = [screen.orientation.type, 0];
    tallest = Math.max(tallest, viewport.height, height);
    if (root.style.getPropertyValue("--viewport-h") === `${height}px`) return;
    glideFeet(() => {
      root.style.setProperty("--viewport-h", `${height}px`);
      root.toggleAttribute("data-keyboard", tallest - height > KEYBOARD_PX);
    }, setOff);
  };
  const holdTop = () => {
    if (window.scrollY !== 0 || viewport.offsetTop !== 0) window.scrollTo(0, 0);
  };
  window.addEventListener("resize", readInsets);
  readInsets();
  addEventListener("octopus-keyboard", (event) => {
    const { height, at } = (event as CustomEvent<{ height: number; at: number }>).detail;
    follow(height, Math.min(at - performance.timeOrigin, performance.now()));
  });
  viewport.addEventListener("resize", () => follow(viewport.height, null));
  viewport.addEventListener("scroll", holdTop);
  window.addEventListener("scroll", holdTop);
  follow(viewport.height, null);
}
