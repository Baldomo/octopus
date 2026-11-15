/** The tab `step` away from `from` in `order`, wrapping around; from no tab, the first going forward and the last back. */
export function cycled(order: readonly string[], from: string | undefined, step: 1 | -1) {
  const at = from === undefined ? -1 : order.indexOf(from);
  if (at < 0) return step === 1 ? order[0] : order.at(-1);
  return order[(at + step + order.length) % order.length];
}
