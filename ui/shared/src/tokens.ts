/** Values from the design tokens in `ui/design/`, read back from the app's stylesheet. */

import { createSignal } from "solid-js";
import type { Swatch } from "./generated";
import { ui } from "./state";

export type Scheme = "light" | "dark";

const darkAppearance = matchMedia("(prefers-color-scheme: dark)");
const schemeNow = (): Scheme => (darkAppearance.matches ? "dark" : "light");
const [osScheme, setOsScheme] = createSignal<Scheme>(schemeNow());
darkAppearance.addEventListener("change", () => setOsScheme(schemeNow()));

/** The scheme every color token resolves against: the client menu's appearance, or the OS's while it picks System. */
export const scheme = (): Scheme => (ui.prefs.appearance === "system" ? osScheme() : ui.prefs.appearance);

export const swatchColor = (swatch: Swatch) => `var(--ws-${swatch.hue}-${swatch.tone})`;

export const token = (name: string) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();

export const colors = (scheme: Scheme) => (name: string) => resolve(name, scheme);

function resolve(name: string, scheme: Scheme): string {
  if (token(name) === "") throw new Error(`no token ${name}`);
  const probe = document.createElement("div");
  probe.style.display = "none";
  probe.style.colorScheme = scheme;
  probe.style.color = `color-mix(in srgb, var(${name}) 100%, transparent)`;
  document.body.append(probe);
  const value = getComputedStyle(probe).color;
  probe.remove();
  return hex(value);
}

/** A `color(srgb r g b / a)` as `#rrggbb` or `#rrggbbaa`, the spelling Monaco and xterm accept. */
function hex(srgb: string): string {
  const channels = srgb.match(/^color\(srgb (\S+) (\S+) ([^\s)]+)(?: \/ ([^\s)]+))?\)$/);
  if (channels === null) throw new Error(`unparsed color ${srgb}`);
  const byte = (unit: string) =>
    Math.round(Math.min(Math.max(Number(unit), 0), 1) * 255)
      .toString(16)
      .padStart(2, "0");
  return `#${channels
    .slice(1)
    .filter((unit) => unit !== undefined)
    .map(byte)
    .join("")}`;
}

export function styleOf(classes: string[], property: "lineHeight" | "paddingTop"): string {
  const root = document.createElement("div");
  root.style.display = "none";
  let leaf = root;
  for (const name of classes) {
    const child = document.createElement("div");
    child.className = name;
    leaf.append(child);
    leaf = child;
  }
  document.body.append(root);
  const value = getComputedStyle(leaf)[property];
  root.remove();
  return value;
}

export const px = (name: string) => Number.parseFloat(token(name));
