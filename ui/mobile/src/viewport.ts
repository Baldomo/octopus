import { report } from "@octopus/shared/state.ts";
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
 * Sizes the app to the part of the screen the on-screen keyboard leaves, so whatever sits at the app's foot sits
 * directly above the keyboard, and marks the root `data-keyboard` while the keyboard is up. The webview scrolls its
 * page to reveal a focused field, which would carry the app under the status bar, so the page is held at the top.
 */
export function followViewport() {
  const viewport = window.visualViewport!;
  let orientation = screen.orientation.type;
  let tallest = 0;
  const follow = () => {
    if (screen.orientation.type !== orientation) [orientation, tallest] = [screen.orientation.type, 0];
    tallest = Math.max(tallest, viewport.height);
    const root = document.documentElement;
    root.style.setProperty("--viewport-h", `${viewport.height}px`);
    root.toggleAttribute("data-keyboard", tallest - viewport.height > KEYBOARD_PX);
    if (window.scrollY !== 0 || viewport.offsetTop !== 0) window.scrollTo(0, 0);
  };
  window.addEventListener("resize", readInsets);
  readInsets();
  viewport.addEventListener("resize", follow);
  viewport.addEventListener("scroll", follow);
  window.addEventListener("scroll", follow);
  follow();
}
