/** How far past a neighbour's middle a held tab's edge travels before the order turns, so a hand resting there holds it. */
const SWAP_SLACK_PX = 4;

/**
 * The index a held tab at `at` takes among the others, whose `middles` stand along the row with the held tab taken out:
 * after each one whose middle stands left of the held tab's `left` edge. That is the moment the held tab's leading edge
 * passes a neighbour's middle as the row shows, in either direction. The index turns only once the edge stands the slack
 * past a middle, so a tab that just made way takes the held tab back only once it passes back as far beyond the middle.
 */
export function slotAmong(middles: readonly number[], left: number, at: number) {
  const before = (edge: number) => middles.filter((middle) => middle < edge).length;
  return Math.min(Math.max(at, before(left - SWAP_SLACK_PX)), before(left + SWAP_SLACK_PX));
}
