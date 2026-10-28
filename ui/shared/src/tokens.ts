/** Values from DESIGN.html, read back from the stylesheet it compiles into. */

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

/** Resolves color tokens in the given scheme, each as `#rrggbb[aa]`. */
export const colors = (scheme: Scheme) => (name: string) => resolve(name, scheme);

function resolve(name: string, scheme: Scheme): string {
  const probe = document.createElement("div");
  probe.style.display = "none";
  probe.style.colorScheme = scheme;
  probe.style.color = `var(${name})`;
  document.body.append(probe);
  const value = getComputedStyle(probe).color;
  probe.remove();
  return hex(value);
}

/** A computed style of an element nested under the given class chain, e.g. `["code-ed", "src"]`. */
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

/** Any CSS color as `#rrggbb` or `#rrggbbaa`, the spelling Monaco and xterm accept. */
function hex(color: string): string {
  const canvas = document.createElement("canvas").getContext("2d");
  if (canvas === null) throw new Error("no 2d canvas to resolve colors");
  canvas.fillStyle = color;
  const resolved = canvas.fillStyle;
  if (resolved.startsWith("#")) return resolved;
  const channels = resolved.match(/[\d.]+/g)?.map(Number) ?? [];
  const [r, g, b, a] = channels;
  if (r === undefined || g === undefined || b === undefined) throw new Error(`unparsed color ${color}: ${resolved}`);
  const byte = (value: number) => Math.round(value).toString(16).padStart(2, "0");
  return `#${byte(r)}${byte(g)}${byte(b)}${a === undefined ? "" : byte(a * 255)}`;
}

export const px = (name: string) => Number.parseFloat(token(name));
