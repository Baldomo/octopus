import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { on } from "@octopus/shared/backend.ts";
import type { BrowserId, Go, OnScreen, TaskRef } from "@octopus/shared/generated.ts";
import { cancel, Format, requestPermissions, scan } from "@tauri-apps/plugin-barcode-scanner";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { Feel } from "./feel";

const INVITE = "octopus://";

export const android = /Android/.test(navigator.userAgent);

export const phone = {
  /** Whether the phone has the name it took on its first pairing. */
  named: () => invoke<boolean>("phone_named"),
  /** Resolves to the key, in hex, of the server an invite pairs; rejects naming the part of the invite that is wrong. */
  inviteKey: (invite: string) => invoke<string>("phone_invite_key", { invite }),
  /** Resolves to the first eight hex digits of the account a link joins; rejects naming the part that is wrong. */
  linkAccount: (link: string) => invoke<string>("phone_link_account", { link }),
  /** Resolves to the paired server's name; a phone that has never paired passes the name it takes. */
  pair: (invite: string, name: string | null) => invoke<string>("phone_pair", { invite, name }),
  /** Joins the account a link names; a phone that has never paired passes the name it takes. */
  join: (link: string, name: string | null) => invoke<void>("phone_join", { link, name }),
  /** The status bar's, the display cutout's and the navigation bar's reach, where the webview leaves them out of the
   * safe area. */
  insets: () => invoke<Insets | null>("phone_insets"),
  browserEngine: () => invoke<boolean>("phone_browser_engine"),
  browserLoad: (at: TaskRef, panel: BrowserId, url: string) => invoke<void>("phone_browser_load", { at, panel, url }),
  browserPlace: (onScreen: OnScreen | null, covered: boolean) =>
    invoke<void>("phone_browser_place", { onScreen, covered }),
  browserGo: (at: TaskRef, panel: BrowserId, go: Go) => invoke<void>("phone_browser_go", { at, panel, go }),
  browserFind: (at: TaskRef, panel: BrowserId, text: string, backwards: boolean) =>
    invoke<void>("phone_browser_find", { at, panel, text, backwards }),
  browserForget: (at: TaskRef, panel: BrowserId) => invoke<void>("phone_browser_forget", { at, panel }),
  /** Whether the system back pops the page's top screen or sheet, which Android then hands to the page. */
  backPops: (pops: boolean) => invoke<void>("phone_back_pops", { pops }),
  feel: (feel: Feel) => invoke<void>("phone_feel", { feel }),
  settings: () => invoke<void>("phone_settings"),
  paste: () => invoke<Pasted>("phone_paste"),
  /** Opens iOS's edit menu, or moves the open one with the same key. */
  editMenu: (menu: EditMenu) => invoke<void>("phone_edit_menu", { menu }),
  closeEditMenu: (key: number) => invoke<void>("phone_edit_menu_close", { key }),
  /** What iOS's edit menu did, by the key of the menu that did it. */
  onEditMenu: (handle: (chose: Chose) => void) => listen<Chose>("phone-edit-menu", (heard) => handle(heard.payload)),
  /** Dials every unconnected server again now, for when the phone's network may have changed. */
  redial: () => invoke<void>("phone_redial"),
  /** Resolves once each of `servers` is reachable again, or after a bounded wait. */
  resume: (servers: string[]) => invoke<void>("phone_resume", { servers }),
  open: openUrl,
};

export const phoneEvents = {
  browserEnded: on<{ task: string; panel: BrowserId }>("browser_ended"),
  browserEdge: on<{ phase: "start" | "move" | "end" | "cancel"; x: number; velocity: number }>("browser_edge"),
};

/** About a box in the OS's points: Copy while there is a selection, whose text it copies, and Paste while `paste`. */
export type EditMenu = {
  key: number;
  x: number;
  y: number;
  width: number;
  height: number;
  selection: string | null;
  paste: boolean;
};

type Chose = { did: "copy"; key: number } | { did: "paste"; key: number; pasted: Pasted };

/** The OS's clipboard: its text, and its images as files on this phone. */
export type Pasted = { text: string; files: string[] };

type Insets = { top: number; left: number; right: number; bottom: number };

/**
 * Asks the OS for the camera, then shows it behind the page, which goes transparent around its finder, and resolves to
 * the text of the first QR code that is an invite or a link.
 */
export async function scanInvite(): Promise<string> {
  if ((await requestPermissions()) !== "granted") {
    throw new Error("octopus may not use the camera; allow it in this phone's settings.");
  }
  for (;;) {
    const { content } = await scan({ windowed: true, formats: [Format.QRCode] });
    if (content.startsWith(INVITE)) return content;
  }
}

export const stopScanning = () => cancel();
