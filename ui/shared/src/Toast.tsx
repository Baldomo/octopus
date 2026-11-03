import { createSignal, type JSX } from "solid-js";

const TOAST_MS = 1800;

const [said, setSaid] = createSignal<{ message: JSX.Element; nth: number } | null>(null);

/** Confirms what an action did, briefly, at the foot of the sidebar or the board. */
export function toast(message: JSX.Element) {
  const nth = (said()?.nth ?? 0) + 1;
  setSaid({ message, nth });
  setTimeout(() => said()?.nth === nth && setSaid(null), TOAST_MS);
}

export const toasting = () => said() !== null;

export const Toast = (props: { ref?: (element: HTMLElement) => void }) => (
  <div ref={props.ref} class="toast" classList={{ on: toasting() }}>
    {said()?.message}
  </div>
);

export async function copy(text: string, what: JSX.Element) {
  await navigator.clipboard.writeText(text);
  toast(<>Copied {what}</>);
}
