/**
 * The index a tab opened from `opener`'s page takes in `order`: right after `opener` and after the tabs it opened that
 * follow it, so tabs opened one after another keep the order they were opened in. A gone opener's tab joins the end.
 */
export function openedAt(order: readonly string[], opener: string, openerOf: (tab: string) => string | undefined) {
  const from = order.indexOf(opener);
  if (from < 0) return order.length;
  let at = from + 1;
  while (at < order.length && openerOf(order[at]!) === opener) at++;
  return at;
}

/** The tab `step` away from `from` in `order`, wrapping around; from no tab, the first going forward and the last back. */
export function cycled(order: readonly string[], from: string | undefined, step: 1 | -1) {
  const at = from === undefined ? -1 : order.indexOf(from);
  if (at < 0) return step === 1 ? order[0] : order.at(-1);
  return order[(at + step + order.length) % order.length];
}
