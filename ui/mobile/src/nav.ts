import { createEffect, createRoot, createSignal } from "solid-js";
import type { FolderPicker } from "@octopus/shared/folderPath.ts";
import type { EntryKind, PtyId } from "@octopus/shared/generated.ts";
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
  | { is: "entry"; key: string; path: string; kind: EntryKind }
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

/**
 * Shows `next` in place of the entries shown. History holds one entry per entry past the root, each holding its index,
 * so entries it adds get history entries of their own, and history goes back over the ones it drops.
 */
function show(next: Entry[]) {
  const from = entries().length;
  for (let index = from; index < next.length; index++) history.pushState(index, "");
  setEntries(next);
  if (next.length < from) history.go(next.length - from);
}

/** Each history entry holds the index of its top entry, so a jump back any number of entries lands on its own. */
addEventListener("popstate", (event) => {
  const index = (event.state as number | null) ?? 0;
  setEntries((shown) => shown.slice(0, index + 1));
});

/** The OS's back pops the top entry while there is one past the root, and leaves the app from the root. */
createRoot(() => createEffect(() => act(phone.backPops(entries().length > 1))));

export const back = () => history.back();

/** The screen a screen sits on: a task's Files and Browser on its Task screen, its editor on its Files. */
function parentOf(screen: Screen): Screen | undefined {
  if (screen.at === "editor") return { at: "files", key: screen.key };
  if (screen.at === "files" || screen.at === "browser") return { at: "task", key: screen.key };
}

const same = (one: Screen, other: Screen) => JSON.stringify(one) === JSON.stringify(other);

/** The screen, after each of its parents up to the first that is `under`. */
function lineage(screen: Screen, under: Screen): Screen[] {
  const parent = parentOf(screen);
  return parent === undefined || same(parent, under) ? [screen] : [...lineage(parent, under), screen];
}

/** Pushes a screen over the parents it lacks under it; from sheets, the screens take their place and every sheet goes. */
export function go(next: Screen) {
  const pushed = lineage(next, screen()).map((each) => ({ screen: each }));
  show([...entries().slice(0, sheetsFrom()), ...pushed]);
}

/** Pops the top screen and every sheet over it. */
export const up = () => show(entries().slice(0, sheetsFrom() - 1));

/** Raises a sheet over the one up, with a history entry of its own, so back lowers it alone. */
export const raiseSheet = (next: Sheet) => show([...entries(), { sheet: next }]);

/** Opens a sheet over the screen, or shows it a level deeper in the sheet that is up. */
export const openSheet = (next: Sheet) => (sheet() === null ? raiseSheet(next) : showInSheet(next, "deeper"));

/** Shows another page in the sheet that is up, without a history entry of its own. */
export const showInSheet = (sheet: Sheet, turned: Toward) =>
  setEntries((shown) => [...shown.slice(0, -1), { sheet, turned }]);

export const home = () => show(entries().slice(0, 1));

/** Opening a task shows it on the Task screen, which focuses it, and makes it the one Tasks tints. */
export function openTask(key: string) {
  go({ at: "task", key });
  markOpened(key);
}

/** Whether the OS refuses the phone its microphone, which a listening terminal it holds then hears as silence. */
export const [micDenied, setMicDenied] = createSignal(false);
