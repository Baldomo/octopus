import { createSignal, Show } from "solid-js";
import { ui } from "@octopus/shared/state.ts";
import { Icon } from "./Icon";
import { Slot } from "./parts";

const SAID_MS = 4000;

const [said, setSaid] = createSignal<{ text: string; nth: number } | null>(null);

/** Confirms what an action did, briefly. */
export function notify(text: string) {
  const nth = (said()?.nth ?? 0) + 1;
  setSaid({ text, nth });
  setTimeout(() => said()?.nth === nth && setSaid(null), SAID_MS);
}

/** A failed op, with its warning, or a confirmation, 8 above the bottom edge and 8 in from the sides. */
export function Toast() {
  return (
    <Show when={ui.problem ?? said()?.text}>
      {(text) => (
        <div class="toast">
          <Show when={ui.problem !== null}>
            <Slot>
              <Icon name="warn" />
            </Slot>
          </Show>
          <span class="name">{text()}</span>
        </div>
      )}
    </Show>
  );
}
