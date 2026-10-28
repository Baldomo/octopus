import { createSignal } from "solid-js";
import { backend } from "@octopus/shared/backend.ts";
import type { TaskRef } from "@octopus/shared/generated.ts";
import { act } from "@octopus/shared/state.ts";

/** What the client menu sheet shows: its own items, one server's menu, or the pairing form. */
export type MenuPage = { show: "menu" } | { show: "server"; name: string } | { show: "pair" };

export const [menuPage, openMenu] = createSignal<MenuPage | null>(null);
export const closeMenu = () => openMenu(null);

/** The key of the task whose sheet is up. */
export const [sheetTask, openTaskSheet] = createSignal<string | null>(null);
export const closeTaskSheet = () => openTaskSheet(null);

/** Whether the OS refuses the phone its microphone, which a listening terminal it holds then hears as silence. */
export const [micDenied, setMicDenied] = createSignal(false);

/** The key of the task on the Task screen; none shows Tasks. */
const [openKey, setOpenKey] = createSignal<string | null>(null);
export { openKey };
export const showTasks = () => setOpenKey(null);

/** Opening a task focuses it, which starts its agent if it has none. */
export function openTask(key: string, at: TaskRef) {
  setOpenKey(key);
  act(backend.taskFocus(at));
}
