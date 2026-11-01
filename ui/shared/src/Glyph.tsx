import { onCleanup, type JSX } from "solid-js";
import type { Glyph as Kind } from "./generated";

export const statusWord: Record<Kind, string> = {
  working: "working",
  waiting: "needs you",
  idle: "idle",
  completed: "completed",
  cancelled: "cancelled",
};

/**
 * Runs a spinner's open quarter once round its still ring, on the document's clock, so every spinner goes round in step
 * and one drawn again keeps its phase. A script animation outlives a hidden ancestor, where a CSS one restarts.
 */
export function turning(ring: SVGCircleElement) {
  const round = ring.pathLength.baseVal;
  const turn = new KeyframeEffect(
    ring,
    { strokeDashoffset: ["0", String(-round)] },
    { duration: 900, iterations: Infinity },
  );
  const animation = new Animation(turn, document.timeline);
  animation.startTime = 0;
  onCleanup(() => animation.cancel());
}

const look: Record<Kind, () => JSX.Element> = {
  working: () => (
    <svg class="spin" viewBox="0 0 14 14">
      <circle cx="7" cy="7" r="3.75" pathLength="100" ref={turning} />
    </svg>
  ),
  waiting: () => <span class="dot needs" />,
  idle: () => <span class="dot idle" />,
  completed: () => <span class="ended">✓</span>,
  cancelled: () => <span class="ended">✕</span>,
};

export function Glyph(props: { glyph: Kind }) {
  return <span class="glyph">{look[props.glyph]()}</span>;
}
