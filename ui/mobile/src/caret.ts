/** How long a field stands still before its caret shows, which iOS takes to bring its caret to where the field stopped. */
const SETTLE_MS = 120;

/** Whether a move in flight carries the focused text field: one on it or on anything holding it. */
const carried = () => {
  const field = document.activeElement;
  return (
    field instanceof HTMLElement &&
    field.matches("input, textarea, [contenteditable]") &&
    document
      .getAnimations()
      .some(
        (move) =>
          move.playState === "running" &&
          move.effect instanceof KeyframeEffect &&
          move.effect.target?.contains(field) === true,
      )
  );
};

/**
 * Marks the root `data-caret-held` while the focused field moves and until it has stood still for `SETTLE_MS`, and the
 * stylesheet hides the caret there. iOS draws the caret natively over the page and catches up with the page's moves
 * only now and then, so a caret riding a move jumps. Every move the page starts, by script or stylesheet, and every
 * focus wakes the watch, which runs frame by frame only while a move carries the focused field.
 */
export function steadyCaret() {
  const root = document.documentElement;
  let frame = 0;
  let settling = 0;
  const look = () => {
    frame = 0;
    if (carried()) {
      root.setAttribute("data-caret-held", "");
      clearTimeout(settling);
      settling = 0;
      frame = requestAnimationFrame(look);
    } else if (root.hasAttribute("data-caret-held") && settling === 0) {
      settling = window.setTimeout(() => {
        settling = 0;
        root.removeAttribute("data-caret-held");
      }, SETTLE_MS);
    }
  };
  const watch = () => frame === 0 && look();
  const animate = Element.prototype.animate;
  Element.prototype.animate = function (...args) {
    const move = animate.apply(this, args);
    watch();
    return move;
  };
  document.addEventListener("focusin", watch);
  document.addEventListener("transitionrun", watch, true);
  document.addEventListener("animationstart", watch, true);
}
