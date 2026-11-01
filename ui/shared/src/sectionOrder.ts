/**
 * Sorts `items` by where their key stands in `order`; an item whose key `order` lacks follows them all, keeping the
 * place it holds in `items`.
 */
export function inOrder<T>(items: readonly T[], order: readonly string[], keyOf: (item: T) => string): T[] {
  const rank = (item: T) => {
    const at = order.indexOf(keyOf(item));
    return at < 0 ? order.length : at;
  };
  return [...items].sort((a, b) => rank(a) - rank(b));
}

export const inSectionOrder = <T extends { name: string }>(items: readonly T[], order: readonly string[]) =>
  inOrder(items, order, (item) => item.name);

/** `names` with `moved` taken out and put back at index `to`. */
export function moveTo(names: readonly string[], moved: string, to: number): string[] {
  const rest = names.filter((name) => name !== moved);
  return [...rest.slice(0, to), moved, ...rest.slice(to)];
}

/**
 * The saved order `saved` with `moved` put just before the section that follows it in `shown`, the order the
 * sections show in after the drag, or last when none does. Every other name keeps its place, shown or not, and a
 * shown section the saved order lacks joins it where it shows.
 */
export function placeAmong(saved: readonly string[], shown: readonly string[], moved: string): string[] {
  const full = [...saved, ...shown.filter((name) => !saved.includes(name))].filter((name) => name !== moved);
  const next = shown[shown.indexOf(moved) + 1];
  const at = next === undefined ? full.length : full.indexOf(next);
  return [...full.slice(0, at), moved, ...full.slice(at)];
}
