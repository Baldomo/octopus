import { createSignal, For, Show, type JSX } from "solid-js";
import { back } from "./nav";
import { Circle, Slot } from "./parts";

/** How far a downward swipe carries the sheet before letting go closes it. */
const SWIPE_CLOSES_PX = 80;

/**
 * Rises from the bottom edge over a scrim and the bottom inset; a swipe down from its top, a tap on the scrim, or the
 * OS's back closes it.
 */
export function Sheet(props: { children: JSX.Element; full?: boolean; style?: JSX.CSSProperties }) {
  let sheet!: HTMLDivElement;
  const [pulled, setPulled] = createSignal(0);
  let start: number | null = null;
  const touchMove = (event: TouchEvent) => {
    const y = event.touches[0]?.clientY;
    if (start === null || y === undefined) return;
    setPulled(Math.max(0, y - start));
    if (pulled() > 0) event.preventDefault();
  };
  const touchEnd = () => {
    if (pulled() > SWIPE_CLOSES_PX) back();
    start = null;
    setPulled(0);
  };
  return (
    <div class="layer">
      <div class="scrim" onClick={back} />
      <div
        ref={(element) => {
          sheet = element;
          element.addEventListener("touchmove", touchMove, { passive: false });
        }}
        class="sheet"
        classList={{ full: props.full, pulled: pulled() > 0 }}
        style={{ ...props.style, transform: pulled() > 0 ? `translateY(${pulled()}px)` : undefined }}
        onTouchStart={(event) => (start = sheet.scrollTop === 0 ? (event.touches[0]?.clientY ?? null) : null)}
        onTouchEnd={touchEnd}
        onTouchCancel={touchEnd}
      >
        <div class="grab" />
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
