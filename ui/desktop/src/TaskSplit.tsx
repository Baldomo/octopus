import { createEffect, createSignal, on, type JSX } from "solid-js";
import { backend } from "@octopus/shared/backend.ts";
import { report, resetSplit, ui } from "@octopus/shared/state.ts";

/** The Task view: one surface left of a divider the pointer drags, the terminal stack right of it. */
export function TaskSplit(props: { shown: boolean; surface: JSX.Element; terminals: JSX.Element }) {
  let split!: HTMLDivElement;
  let divider!: HTMLDivElement;
  const [held, setHeld] = createSignal(false);
  /** The share the pointer drags to, shown until the app's state holds it. */
  const [dragged, setDragged] = createSignal<number | null>(null);
  createEffect(
    on(
      () => ui.version,
      () => ui.prefs.split === dragged() && setDragged(null),
      { defer: true },
    ),
  );

  /** Where the surface's column starts and how wide both columns are together. */
  const columns = () => {
    const style = getComputedStyle(split);
    const padding = parseFloat(style.paddingLeft) + parseFloat(style.paddingRight);
    const left = split.getBoundingClientRect().left + parseFloat(style.paddingLeft);
    return { left, width: split.clientWidth - padding - divider.offsetWidth };
  };
  const shareAt = (x: number) => {
    const { left, width } = columns();
    return Math.min(1, Math.max(0, (x - left - divider.offsetWidth / 2) / width));
  };
  /** The minimums can hold the divider short of the pointer, so the drag keeps the share it shows. */
  const shown = () => {
    const { left, width } = columns();
    return (divider.getBoundingClientRect().left - left) / width;
  };
  const release = () => {
    setHeld(false);
    if (dragged() === null) return;
    const share = shown();
    setDragged(share);
    backend.uiAct({ do: "set_split", split: share }).catch((error) => {
      setDragged(null);
      report(error);
    });
  };

  return (
    <div
      ref={split}
      class="task-split"
      classList={{ resizing: held() }}
      style={{ display: props.shown ? "" : "none", "--split": String(dragged() ?? ui.prefs.split) }}
    >
      {props.surface}
      <div
        ref={divider}
        class="splitter"
        onPointerDown={(event) => {
          if (event.button !== 0) return;
          divider.setPointerCapture(event.pointerId);
          setHeld(true);
        }}
        onPointerMove={(event) => held() && setDragged(shareAt(event.clientX))}
        onLostPointerCapture={release}
        onDblClick={resetSplit}
      />
      {props.terminals}
    </div>
  );
}
