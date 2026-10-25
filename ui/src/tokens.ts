/** Values from DESIGN.html, read back from the stylesheet it compiles into. */

export const token = (name: string) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();

/** A computed style of an element nested under the given class chain, e.g. `["code-ed", "ln"]`. */
export function styleOf(classes: string[], property: "color" | "lineHeight" | "paddingTop"): string {
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
export function hex(color: string): string {
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
