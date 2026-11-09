import { createEffect, createRoot, createSignal } from "solid-js";
import { backend } from "@octopus/shared/backend.ts";
import type { PtyId, TaskRef } from "@octopus/shared/generated.ts";
import { act, openTask as markOpened } from "@octopus/shared/state.ts";
import { phone } from "./phone";

/** What the user is reading the camera or clipboard for: an invite pairs a server, a link joins an account. */
export type Reading = "invite" | "link";

export type Screen =
  | { at: "tasks" }
  | { at: "task"; key: string }
  | { at: "scan"; reading: Reading }
  | { at: "board"; section: string }
  | { at: "files"; key: string }
  | { at: "editor"; key: string }
  | { at: "browser"; key: string }
  | { at: "search" };

export type Sheet =
  | { is: "client" }
  | { is: "server"; name: string }
  | { is: "pair"; invite: string; reading: Reading }
  | { is: "task"; key: string; open: { panel: PtyId | null } | null }
  | { is: "reopen"; key: string }
  | { is: "section"; name: string }
  | { is: "addws" }
  | { is: "newtask"; section: string | null };

/** Where a page shown in a sheet in place of another goes: a level deeper, back up one, or nowhere, as its same page. */
export type Toward = "deeper" | "back" | "here";

type SheetEntry = { sheet: Sheet; toward: Toward };
type Entry = { screen: Screen } | SheetEntry;

/**
 * The screens and sheets shown, root first. Each entry past the root has its own history entry, so the OS's back
 * gesture and back button pop the top one, and every pop in the app goes through history to stay in step with it.
 */
const [entries, setEntries] = createSignal<Entry[]>([{ screen: { at: "tasks" } }]);

export const screens = () => entries().flatMap((entry) => ("screen" in entry ? [entry.screen] : []));
export const screen = () => screens().at(-1)!;
export const sheetEntry = (): SheetEntry | null => {
  const top = entries().at(-1)!;
  return "sheet" in top ? top : null;
};
export const sheet = () => sheetEntry()?.sheet ?? null;

const push = (entry: Entry) => {
  history.pushState(entries().length, "");
  setEntries((shown) => [...shown, entry]);
};

/** Each history entry holds the index of its top entry, so a jump back any number of entries lands on its own. */
addEventListener("popstate", (event) => {
  const index = (event.state as number | null) ?? 0;
  setEntries((shown) => shown.slice(0, index + 1));
});

/** The OS's back pops the top entry while there is one past the root, and leaves the app from the root. */
createRoot(() => createEffect(() => act(phone.backPops(entries().length > 1))));

export const back = () => history.back();

/** Pushes a screen; from a sheet, the screen takes the sheet's place. */
export function go(screen: Screen) {
  if (sheet() === null) return push({ screen });
  setEntries((shown) => [...shown.slice(0, -1), { screen }]);
}

export function openSheet(next: Sheet) {
  if (sheet() !== null) return showInSheet(next, "deeper");
  push({ sheet: next, toward: "here" });
}

/** Shows another page in the sheet that is up, without a history entry of its own. */
export const showInSheet = (sheet: Sheet, toward: Toward) =>
  setEntries((shown) => [...shown.slice(0, -1), { sheet, toward }]);

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
