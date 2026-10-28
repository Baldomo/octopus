import { createEffect, createSignal, on, onCleanup } from "solid-js";
import { backend } from "./backend";
import type { Match, TaskRef } from "./generated";
import { report } from "./state";

const SEARCH_DELAY_MS = 150;
/** File search starts at this many characters; shorter patterns match nearly every line. */
const MIN_QUERY = 2;

/**
 * The lines of a task's files that hold `query`, searched a moment after the typing stops and only from MIN_QUERY
 * characters on.
 */
export function fileMatches(at: () => TaskRef | undefined, query: () => string, most: number) {
  const [matches, setMatches] = createSignal<Match[]>([]);
  createEffect(
    on([() => query().trim(), at], ([text, task]) => {
      setMatches([]);
      if (text.length < MIN_QUERY || task === undefined) return;
      const timer = setTimeout(() => {
        backend.fileSearch(task, text).then((found) => {
          if (query().trim() === text) setMatches(found.slice(0, most));
        }, report);
      }, SEARCH_DELAY_MS);
      onCleanup(() => clearTimeout(timer));
    }),
  );
  return matches;
}
