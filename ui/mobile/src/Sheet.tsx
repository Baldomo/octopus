import { createSignal, Show, type JSX } from "solid-js";

/** How far a downward swipe carries the sheet before letting go closes it. */
const SWIPE_CLOSES_PX = 80;

/** Rises from the foot of the screen; a downward swipe from its top, or a tap above it, closes it. */
export function Sheet(props: { shown: boolean; close: () => void; children: JSX.Element }) {
  let sheet!: HTMLDivElement;
  const [pulled, setPulled] = createSignal(0);
  let start: number | null = null;
  const touchStart = (event: TouchEvent) => {
    start = sheet.scrollTop === 0 ? (event.touches[0]?.clientY ?? null) : null;
  };
  const touchMove = (event: TouchEvent) => {
    const y = event.touches[0]?.clientY;
    if (start === null || y === undefined) return;
    setPulled(Math.max(0, y - start));
    if (pulled() > 0) event.preventDefault();
  };
  const touchEnd = () => {
    if (pulled() > SWIPE_CLOSES_PX) props.close();
    start = null;
    setPulled(0);
  };
  return (
    <Show when={props.shown}>
      <div class="scrim" onClick={props.close} />
      <div
        ref={(element) => {
          sheet = element;
          element.addEventListener("touchmove", touchMove, { passive: false });
        }}
        class="sheet"
        classList={{ pulled: pulled() > 0 }}
        style={{ transform: pulled() > 0 ? `translateY(${pulled()}px)` : undefined }}
        onTouchStart={touchStart}
        onTouchEnd={touchEnd}
        onTouchCancel={touchEnd}
      >
        <div class="grabber" />
        {props.children}
      </div>
    </Show>
  );
}

/** A sheet page's header: a back chevron when the page has a parent, its title, and lines beneath. */
export function SheetHead(props: { title: JSX.Element; lines?: string[]; back?: () => void }) {
  return (
    <div class="sh">
      <Show when={props.back}>
        {(back) => (
          <button class="back" aria-label="Back" onClick={back()}>
            ‹
          </button>
        )}
      </Show>
      <div class="sh-text">
        <b>{props.title}</b>
        {props.lines?.map((line) => (
          <span class="mono">{line}</span>
        ))}
      </div>
    </div>
  );
}
