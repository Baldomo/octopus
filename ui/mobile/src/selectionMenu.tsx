import { createEffect, createSignal, type JSX, onCleanup, Show } from "solid-js";
import { act } from "@octopus/shared/state.ts";
import { observeSize } from "./parts";
import { android, type EditMenu, type Pasted, phone } from "./phone";
import { copy, notify } from "./Toast";

type Size = { width: number; height: number };
export type Box = { x: number; y: number; width: number; height: number };

/** What a terminal's selection menu offers, where it shows, and what its choices do. */
export type MenuProps = {
  /** The selected text, which Copy copies, while there is one. */
  selection: string | null;
  paste: boolean;
  /** Where the page's own menu of `size` sits in the pane, hidden until it is measured. */
  place: (size: Size | null) => JSX.CSSProperties;
  /** What the menu points at, in the OS's points. */
  box: () => Box;
  /** Clears the selection and the menu. */
  done: () => void;
  pasted: (pasted: Pasted) => void;
};

let opened = 0;

/** iOS's own edit menu, whose Paste reads the clipboard with nothing more to confirm. */
function SystemMenu(props: MenuProps) {
  const key = ++opened;
  let sent = "";
  createEffect(() => {
    const menu: EditMenu = { key, ...props.box(), selection: props.selection, paste: props.paste };
    const shape = JSON.stringify(menu);
    if (shape === sent) return;
    sent = shape;
    act(phone.editMenu(menu));
  });
  const heard = phone.onEditMenu((chose) => {
    if (chose.key !== key) return;
    props.done();
    if (chose.did === "copy") notify("Copied the selection");
    else props.pasted(chose.pasted);
  });
  onCleanup(() => {
    act(phone.closeEditMenu(key));
    act(heard.then((unlisten) => unlisten()));
  });
  return null;
}

function PageMenu(props: MenuProps) {
  const [size, setSize] = createSignal<Size | null>(null);
  return (
    <div ref={(menu) => observeSize(menu, setSize)} class="selmenu" style={props.place(size())}>
      <Show when={props.selection}>
        {(text) => <button onClick={() => act(copy(text(), "the selection").then(props.done))}>Copy</button>}
      </Show>
      <Show when={props.paste}>
        <button
          onClick={() => {
            props.done();
            act(phone.paste().then(props.pasted));
          }}
        >
          Paste
        </button>
      </Show>
    </div>
  );
}

/** A terminal's selection menu: iOS's own edit menu, else one the page draws. */
export const SelectionMenu = android ? PageMenu : SystemMenu;
