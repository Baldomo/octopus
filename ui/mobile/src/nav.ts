import { createEffect, createRoot, createSignal } from "solid-js";
import type { FolderPicker } from "@octopus/shared/folderPath.ts";
import type { PtyId } from "@octopus/shared/generated.ts";
import type { Toward } from "@octopus/shared/motion.ts";
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
  | { is: "newtask"; section: string | null }
  | { is: "folder"; picker: FolderPicker };

/** A sheet up, and where it turned from the page it was shown in place of, if it was. */
export type SheetEntry = { sheet: Sheet; turned?: Toward };
type Entry = { screen: Screen } | SheetEntry;

/**
 * The screens and sheets shown, root first. Each entry past the root has its own history entry, so the OS's back
 * gesture and back button pop the top one, and every pop in the app goes through history to stay in step with it.
 */
const [entries, setEntries] = createSignal<Entry[]>([{ screen: { at: "tasks" } }]);

export const screens = () => entries().flatMap((entry) => ("screen" in entry ? [entry.screen] : []));
export const screen = () => screens().at(-1)!;
/** Where the sheets over the top screen start among the entries. */
const sheetsFrom = () => entries().findLastIndex((entry) => "screen" in entry) + 1;
/** The sheets up over the top screen, the lowest first. */
export const sheetEntries = (): SheetEntry[] =>
  entries()
    .slice(sheetsFrom())
    .flatMap((entry) => ("sheet" in entry ? [entry] : []));
export const sheet = () => sheetEntries().at(-1)?.sheet ?? null;

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

/**
 * Pushes a screen; from sheets, the screen takes the lowest one's place and every sheet goes. The history entries of
 * the sheets above the lowest go back with them, landing on the lowest's, which the screen takes over.
 */
export function go(screen: Screen) {
  const sheets = sheetEntries().length;
  if (sheets === 0) return push({ screen });
  setEntries((shown) => [...shown.slice(0, sheetsFrom()), { screen }]);
  if (sheets > 1) history.go(1 - sheets);
}

/** Raises a sheet over the one up, with a history entry of its own, so back lowers it alone. */
export const raiseSheet = (next: Sheet) => push({ sheet: next });

/** Opens a sheet over the screen, or shows it a level deeper in the sheet that is up. */
export const openSheet = (next: Sheet) => (sheet() === null ? raiseSheet(next) : showInSheet(next, "deeper"));

/** Shows another page in the sheet that is up, without a history entry of its own. */
export const showInSheet = (sheet: Sheet, turned: Toward) =>
  setEntries((shown) => [...shown.slice(0, -1), { sheet, turned }]);

export function home() {
  const depth = entries().length - 1;
  if (depth > 0) history.go(-depth);
}

/** Opening a task shows it on the Task screen, which focuses it, and makes it the one Tasks tints. */
export function openTask(key: string) {
  go({ at: "task", key });
  markOpened(key);
}

/** Whether the OS refuses the phone its microphone, which a listening terminal it holds then hears as silence. */
export const [micDenied, setMicDenied] = createSignal(false);
