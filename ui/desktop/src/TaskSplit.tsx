import { createEffect, createSignal, on, type JSX } from "solid-js";
import { backend } from "@octopus/shared/backend.ts";
import { act, report, resetSplit, ui } from "@octopus/shared/state.ts";
import { desktop } from "./backend";
import { holds, MIDDLE } from "./detent";

/** The Task view: one surface left of a divider the pointer drags, the terminal stack right of it. */
export function TaskSplit(props: { shown: boolean; surface: JSX.Element; terminals: JSX.Element }) {
  let split!: HTMLDivElement;
  let divider!: HTMLDivElement;
  const [held, setHeld] = createSignal(false);
  const [centered, setCentered] = createSignal(false);
  /** The share the pointer drags to, shown until the app's state holds it. */
  const [dragged, setDragged] = createSignal<number | null>(null);
  createEffect(
    on(
      () => ui.version,
      () => ui.prefs.split === dragged() && setDragged(null),
      { defer: true },
    ),
  );

  /** Where the surface's column starts, both columns' width, and whether the minimums leave the middle open. */
  const columns = () => {
    const style = getComputedStyle(split);
    const px = (property: string) => parseFloat(style.getPropertyValue(property));
    const left = split.getBoundingClientRect().left + px("padding-left");
    const width = split.clientWidth - px("padding-left") - px("padding-right") - divider.offsetWidth;
    return { left, width, middleOpen: width / 2 >= Math.max(px("--surface-min"), px("--terminals-min")) };
  };
  const drag = (x: number) => {
    const { left, width, middleOpen } = columns();
    const share = Math.min(1, Math.max(0, (x - left - divider.offsetWidth / 2) / width));
    const catches = middleOpen && holds((share - MIDDLE) * width, centered());
    if (catches && !centered()) act(desktop.hapticAlignment());
    setCentered(catches);
    setDragged(catches ? MIDDLE : share);
  };
  /** The minimums can hold the divider short of the pointer, so the drag keeps the share it shows. */
  const shown = () => {
    const { left, width } = columns();
    return (divider.getBoundingClientRect().left - left) / width;
  };
  const release = () => {
    setHeld(false);
    if (dragged() === null) return;
    const share = centered() ? MIDDLE : shown();
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
        classList={{ centered: centered() }}
        onPointerDown={(event) => {
          if (event.button !== 0) return;
          divider.setPointerCapture(event.pointerId);
          setCentered(columns().middleOpen && (dragged() ?? ui.prefs.split) === MIDDLE);
          setHeld(true);
        }}
        onPointerMove={(event) => held() && drag(event.clientX)}
        onLostPointerCapture={release}
        onDblClick={resetSplit}
      />
      {props.terminals}
    </div>
  );
}
