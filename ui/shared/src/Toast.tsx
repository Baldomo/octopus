import { createSignal, type JSX } from "solid-js";

const TOAST_MS = 1800;

const [shown, setShown] = createSignal<{ message: JSX.Element; nth: number } | null>(null);

export function toast(message: JSX.Element) {
  const nth = (shown()?.nth ?? 0) + 1;
  setShown({ message, nth });
  setTimeout(() => shown()?.nth === nth && setShown(null), TOAST_MS);
}

/** What a failure says, whatever was thrown. */
export const said = (error: unknown) => (error instanceof Error ? error.message : String(error));

/** Runs `op` on each item in turn, toasting why each failed one did; resolves to how many succeeded. */
export async function each<T>(items: T[], op: (item: T) => Promise<unknown>) {
  let done = 0;
  for (const item of items) {
    try {
      await op(item);
      done++;
    } catch (error) {
      toast(said(error));
    }
  }
  return done;
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
