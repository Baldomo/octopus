import { createSignal } from "solid-js";
import { backend } from "@octopus/shared/backend.ts";
import type { TaskRef } from "@octopus/shared/generated.ts";
import { act, openTask as markOpened } from "@octopus/shared/state.ts";

/** A full screen the stack pushes; Tasks is its root. */
export type Screen =
  | { at: "tasks" }
  | { at: "task"; key: string }
  | { at: "scan" }
  | { at: "board"; section: string }
  | { at: "files"; key: string }
  | { at: "editor"; key: string }
  | { at: "browser"; key: string }
  | { at: "search" };

/** A sheet rising over the screen beneath it. */
export type Sheet =
  | { is: "client" }
  | { is: "server"; name: string }
  | { is: "pair"; invite: string }
  | { is: "task"; key: string; open: boolean }
  | { is: "reopen"; key: string }
  | { is: "section"; name: string }
  | { is: "addws" }
  | { is: "newtask"; section: string | null };

type Entry = { screen: Screen } | { sheet: Sheet };

/**
 * The screens and sheets shown, root first. Each entry past the root has its own history entry, so the OS's back
 * gesture and back button pop the top one, and every pop in the app goes through history to stay in step with it.
 */
const [entries, setEntries] = createSignal<Entry[]>([{ screen: { at: "tasks" } }]);

export const screens = () => entries().flatMap((entry) => ("screen" in entry ? [entry.screen] : []));
export const screen = () => screens().at(-1)!;
export const sheet = () => {
  const top = entries().at(-1)!;
  return "sheet" in top ? top.sheet : null;
};

const push = (entry: Entry) => {
  history.pushState(entries().length, "");
  setEntries((shown) => [...shown, entry]);
};

/**
 * Each history entry holds the index of its top entry, so a jump back any number of entries lands on its own. A step
 * forward, which iOS's edge swipe offers, finds no entry left to show and steps back again.
 */
addEventListener("popstate", (event) => {
  const index = (event.state as number | null) ?? 0;
  const top = entries().length - 1;
  if (index > top) return history.go(top - index);
  setEntries((shown) => shown.slice(0, index + 1));
});

export const back = () => history.back();

/** Pushes a screen; from a sheet, the screen takes the sheet's place. */
export function go(screen: Screen) {
  if (sheet() === null) return push({ screen });
  setEntries((shown) => [...shown.slice(0, -1), { screen }]);
}

export function openSheet(sheet: Sheet) {
  if ("sheet" in entries().at(-1)!) return showInSheet(sheet);
  push({ sheet });
}

/** Shows another page in the sheet that is up, without a history entry of its own. */
export const showInSheet = (sheet: Sheet) => setEntries((shown) => [...shown.slice(0, -1), { sheet }]);

/** Pops every screen and sheet back to Tasks. */
export function home() {
  const depth = entries().length - 1;
  if (depth > 0) history.go(-depth);
}

/** Opening a task focuses it, which starts its agent if it has none, and makes it the one Tasks tints. */
export function openTask(key: string, at: TaskRef) {
  go({ at: "task", key });
  markOpened(key);
  act(backend.taskFocus(at));
}

/** Whether the OS refuses the phone its microphone, which a listening terminal it holds then hears as silence. */
export const [micDenied, setMicDenied] = createSignal(false);
