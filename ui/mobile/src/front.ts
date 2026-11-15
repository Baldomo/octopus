import { batch, createSignal } from "solid-js";
import { act, servers } from "@octopus/shared/state.ts";
import { windowFocused } from "@octopus/shared/terminalSession.ts";
import { android, phone } from "./phone";

const [inFront, setInFront] = createSignal(true);
/** From coming back to the front until each server reachable when the app left is reached again, or given up on. */
const [resuming, setResuming] = createSignal(false);

/** The app shows its screens: in front, and done reaching again the servers it left reachable. */
export const present = () => inFront() && !resuming();

let reachableWhenLeft = new Set<string>();
let resumes = 0;

/** Whether the server was reachable when the app left the front and the app has not yet reached it again. */
export const settles = (server: string) => resuming() && reachableWhenLeft.has(server);

function front(shown: boolean) {
  if (!shown) {
    setInFront(false);
    reachableWhenLeft = new Set(
      servers().flatMap((server) => (server.reachable || settles(server.name) ? [server.name] : [])),
    );
    return;
  }
  const resume = ++resumes;
  batch(() => {
    setInFront(true);
    setResuming(true);
  });
  act(
    phone.resume([...reachableWhenLeft]).finally(() => {
      if (resume !== resumes) return;
      setResuming(false);
      if (inFront()) windowFocused();
    }),
  );
}

/**
 * The app is in front while its window is visible, overlays and its own permission prompts included. On iOS the
 * page's visibility follows the scene; Android hides the page for a prompt too, so the app's activity says instead.
 */
export function followFront() {
  if (android) {
    window.addEventListener("octopus-front", (event) => front((event as CustomEvent<boolean>).detail));
    return;
  }
  document.addEventListener("visibilitychange", () => front(document.visibilityState === "visible"));
}
