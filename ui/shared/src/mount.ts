import { catchError, type JSX } from "solid-js";
import { render } from "solid-js/web";
import { report } from "./state";

/** Renders the app; a failure in its first render fails the mount, and one after shows as a problem. */
export function mount(app: () => JSX.Element, root: HTMLElement) {
  let mounted = false;
  render(
    () =>
      catchError(app, (error) => {
        if (!mounted) throw error;
        report(error);
      }),
    root,
  );
  mounted = true;
}
