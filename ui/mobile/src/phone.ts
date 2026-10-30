import { invoke } from "@tauri-apps/api/core";
import { cancel, Format, scan } from "@tauri-apps/plugin-barcode-scanner";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { Feel } from "./feel";

const INVITE = "octopus://";

/** The phone app's own commands, beside the client app core's. */
export const phone = {
  /** Whether the phone has the name it took on its first pairing. */
  named: () => invoke<boolean>("phone_named"),
  /** Resolves to the key, in hex, of the server an invite pairs; rejects naming the part of the invite that is wrong. */
  inviteKey: (invite: string) => invoke<string>("phone_invite_key", { invite }),
  /** Resolves to the paired server's name; a phone that has never paired passes the name it takes. */
  pair: (invite: string, name: string | null) => invoke<string>("phone_pair", { invite, name }),
  /** The status bar's, the display cutout's and the navigation bar's reach, where the webview leaves them out of the
   * safe area. */
  insets: () => invoke<Insets | null>("phone_insets"),
  /** Shows a page of `server`'s task, holding the tunnel to its port while it is a loopback page. */
  browse: (server: string, url: string) => invoke<Page>("phone_browse", { server, url }),
  /** Lets go of the shown page's tunnel. */
  unbrowse: () => invoke<void>("phone_unbrowse"),
  /** Whether the system back pops the page's top screen or sheet, which Android then hands to the page. */
  backPops: (pops: boolean) => invoke<void>("phone_back_pops", { pops }),
  /** Plays a haptic as the OS's own. */
  feel: (feel: Feel) => invoke<void>("phone_feel", { feel }),
  /** Opens this app's page of the OS's settings, where a denied microphone is granted. */
  settings: () => invoke<void>("phone_settings"),
  /** Opens an http or https URL in the OS's browser. */
  open: (url: string) => openUrl(url),
};

/** A page as the phone shows it, and whether it reaches the server's port through the tunnel. */
export type Page = { url: string; tunneled: boolean };

export type Insets = { top: number; left: number; right: number; bottom: number };

/**
 * Shows the camera behind the page, which goes transparent around its finder, and resolves to the text of the first
 * QR code that is an invite.
 */
export async function scanInvite(): Promise<string> {
  for (;;) {
    const { content } = await scan({ windowed: true, formats: [Format.QRCode] });
    if (content.startsWith(INVITE)) return content;
  }
}

export const stopScanning = () => cancel();
