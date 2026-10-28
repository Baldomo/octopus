import { invoke } from "@tauri-apps/api/core";
import { Format, scan } from "@tauri-apps/plugin-barcode-scanner";

const INVITE = "octopus://";

/** The phone app's own commands, beside the client app core's. */
export const phone = {
  /** Whether the phone has the name it took on its first pairing. */
  named: () => invoke<boolean>("phone_named"),
  /** Resolves to the paired server's name; a phone that has never paired passes the name it takes. */
  pair: (invite: string, name: string | null) => invoke<string>("phone_pair", { invite, name }),
  /** The status bar's and the display cutout's reach, where the webview leaves them out of the safe area. */
  insets: () => invoke<Insets | null>("phone_insets"),
};

export type Insets = { top: number; left: number; right: number };

/** Opens the camera and resolves to the text of the first QR code that is an invite. */
export async function scanInvite(): Promise<string> {
  for (;;) {
    const { content } = await scan({ windowed: false, formats: [Format.QRCode] });
    if (content.startsWith(INVITE)) return content;
  }
}
