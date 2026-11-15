import { createSignal, type JSX } from "solid-js";

const TOAST_MS = 1800;

const [shown, setShown] = createSignal<{ message: JSX.Element; nth: number } | null>(null);

export function toast(message: JSX.Element) {
  const nth = (shown()?.nth ?? 0) + 1;
  setShown({ message, nth });
  setTimeout(() => shown()?.nth === nth && setShown(null), TOAST_MS);
}

export const toasting = () => shown() !== null;

export const Toast = (props: { ref?: (element: HTMLElement) => void }) => (
  <div ref={props.ref} class="toast" classList={{ on: toasting() }}>
    {shown()?.message}
  </div>
);

export async function copy(text: string, what: JSX.Element) {
  await navigator.clipboard.writeText(text);
  toast(<>Copied {what}</>);
}
